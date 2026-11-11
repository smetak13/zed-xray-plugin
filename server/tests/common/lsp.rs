use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct Lens {
    pub line: u32,
    pub title: String,
    pub command: String,
    pub arguments: Vec<Value>,
}

type Log = Arc<Mutex<Vec<(String, Value)>>>;

pub struct LspClient {
    child: Child,
    writer: Arc<tokio::sync::Mutex<ChildStdin>>,
    responses: mpsc::UnboundedReceiver<Value>,
    log: Log,
    next_id: u64,
    pub uri: String,
    pub initialize_result: Value,
}

async fn write_message(writer: &tokio::sync::Mutex<ChildStdin>, message: &Value) {
    let body = message.to_string();
    let mut writer = writer.lock().await;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
        .await
        .unwrap();
    writer.flush().await.unwrap();
}

async fn read_message(reader: &mut BufReader<tokio::process::ChildStdout>) -> Option<Value> {
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await.ok()? == 0 {
            return None;
        }
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length: ") {
            length = value.parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.ok()?;
    serde_json::from_slice(&body).ok()
}

impl LspClient {
    pub async fn start(options: Value) -> LspClient {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_xray"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let writer = Arc::new(tokio::sync::Mutex::new(child.stdin.take().unwrap()));
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let (sender, responses) = mpsc::unbounded_channel();
        let log: Log = Arc::default();
        let reader_log = log.clone();
        let reader_writer = writer.clone();
        tokio::spawn(async move {
            while let Some(message) = read_message(&mut reader).await {
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .map(String::from);
                match (method, message.get("id")) {
                    (Some(method), Some(id)) => {
                        reader_log
                            .lock()
                            .unwrap()
                            .push((method.clone(), message["params"].clone()));
                        let result = if method == "workspace/applyEdit" {
                            json!({ "applied": true })
                        } else {
                            Value::Null
                        };
                        let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
                        write_message(&reader_writer, &reply).await;
                    }
                    (Some(method), None) => {
                        reader_log
                            .lock()
                            .unwrap()
                            .push((method, message["params"].clone()));
                    }
                    _ => {
                        let _ = sender.send(message);
                    }
                }
            }
        });
        let mut client = LspClient {
            child,
            writer,
            responses,
            log,
            next_id: 1,
            uri: String::new(),
            initialize_result: Value::Null,
        };
        let root = std::env::temp_dir();
        let result = client
            .request(
                "initialize",
                json!({
                    "processId": null,
                    "rootUri": format!("file://{}", root.display()),
                    "capabilities": {},
                    "initializationOptions": options
                }),
            )
            .await;
        client.initialize_result = result;
        client.notify("initialized", json!({})).await;
        client
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        write_message(
            &self.writer,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
        loop {
            let response = tokio::time::timeout(Duration::from_secs(10), self.responses.recv())
                .await
                .expect("timed out waiting for a response")
                .expect("server closed the connection");
            if response["id"] == json!(id) {
                assert!(response.get("error").is_none(), "{response}");
                return response["result"].clone();
            }
        }
    }

    pub async fn notify(&mut self, method: &str, params: Value) {
        write_message(
            &self.writer,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
        .await;
    }

    pub async fn open(&mut self, path: &std::path::Path, text: &str) {
        self.uri = format!("file://{}", path.display());
        let uri = self.uri.clone();
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "python", "version": 1, "text": text } }),
        )
        .await;
    }

    pub async fn change(&mut self, range: (u32, u32, u32, u32), text: &str) {
        let uri = self.uri.clone();
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "range": lsp_range(range), "text": text }]
            }),
        )
        .await;
    }

    pub async fn save(&mut self) {
        let uri = self.uri.clone();
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    }

    pub async fn command(&mut self, command: &str, arguments: Vec<Value>) -> Value {
        self.request(
            "workspace/executeCommand",
            json!({ "command": command, "arguments": arguments }),
        )
        .await
    }

    pub async fn code_actions(&mut self, line: u32) -> Vec<(String, String, Vec<Value>)> {
        let uri = self.uri.clone();
        let result = self
            .request(
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": uri },
                    "range": lsp_range((line, 0, line, 0)),
                    "context": { "diagnostics": [] }
                }),
            )
            .await;
        result
            .as_array()
            .unwrap()
            .iter()
            .map(|action| {
                let command = &action["command"];
                (
                    action["title"].as_str().unwrap().to_string(),
                    command["command"].as_str().unwrap().to_string(),
                    command["arguments"].as_array().cloned().unwrap_or_default(),
                )
            })
            .collect()
    }

    pub async fn run_at(&mut self, line: u32) {
        let (_, command, arguments) = self
            .code_actions(line)
            .await
            .into_iter()
            .find(|action| action.0 == "xray: Run")
            .expect("xray: Run is offered");
        assert_eq!(self.command(&command, arguments).await, Value::Null);
    }

    pub async fn lenses(&mut self) -> Vec<Lens> {
        let uri = self.uri.clone();
        let result = self
            .request(
                "textDocument/codeLens",
                json!({ "textDocument": { "uri": uri } }),
            )
            .await;
        result
            .as_array()
            .unwrap()
            .iter()
            .map(|lens| Lens {
                line: lens["range"]["start"]["line"].as_u64().unwrap() as u32,
                title: lens["command"]["title"].as_str().unwrap().to_string(),
                command: lens["command"]["command"].as_str().unwrap().to_string(),
                arguments: lens["command"]["arguments"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect()
    }

    pub async fn lenses_until(&mut self, accept: impl Fn(&[Lens]) -> bool) -> Vec<Lens> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let lenses = self.lenses().await;
            if accept(&lenses) {
                return lenses;
            }
            assert!(
                Instant::now() < deadline,
                "lenses never matched, last seen: {:?}",
                titles(&lenses)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn lenses_with(&mut self, line: u32, title: &str) -> Vec<Lens> {
        self.lenses_until(|lenses| {
            lenses
                .iter()
                .any(|lens| lens.line == line && lens.title == title)
        })
        .await
    }

    pub async fn click(&mut self, lens: &Lens) {
        let result = self.command(&lens.command, lens.arguments.clone()).await;
        assert_eq!(result, Value::Null);
    }

    pub fn received(&self, method: &str) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == method)
            .map(|(_, params)| params.clone())
            .collect()
    }

    pub async fn received_until(&self, method: &str) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let messages = self.received(method);
            if !messages.is_empty() {
                return messages;
            }
            assert!(Instant::now() < deadline, "never received {method}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn shutdown(mut self) {
        self.request("shutdown", Value::Null).await;
        self.notify("exit", Value::Null).await;
        let _ = self.child.wait().await;
    }
}

pub fn lsp_range(range: (u32, u32, u32, u32)) -> Value {
    json!({
        "start": { "line": range.0, "character": range.1 },
        "end": { "line": range.2, "character": range.3 }
    })
}

pub fn titles(lenses: &[Lens]) -> Vec<(u32, String)> {
    lenses
        .iter()
        .map(|lens| (lens.line, lens.title.clone()))
        .collect()
}

pub fn find<'a>(lenses: &'a [Lens], line: u32, title: &str) -> &'a Lens {
    lenses
        .iter()
        .find(|lens| lens.line == line && lens.title == title)
        .unwrap_or_else(|| panic!("no lens {title:?} on line {line}: {:?}", titles(lenses)))
}
