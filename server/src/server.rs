use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower_lsp_server::jsonrpc::{Error, Result};
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability,
    CodeActionResponse, CodeLens, CodeLensOptions, CodeLensParams, Command,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    ExecuteCommandOptions, ExecuteCommandParams, InitializeParams, InitializeResult, MessageType,
    Position, Range, ServerCapabilities, ShowDocumentParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Uri,
    WorkspaceEdit,
};
use tower_lsp_server::{Client, LanguageServer};

use crate::config::Config;
use crate::text;
use crate::trace::{Outcome, Stop, Template};
use crate::tracer::{self, Language, TraceRequest};
use crate::view::{Action, View};

const RUN: &str = "xray.run";
const CLEAR: &str = "xray.clear";
const PREV_ITERATION: &str = "xray.prevIteration";
const NEXT_ITERATION: &str = "xray.nextIteration";
const PREV_CALL: &str = "xray.prevCall";
const NEXT_CALL: &str = "xray.nextCall";
const NOOP: &str = "xray.noop";
const COMMANDS: [&str; 7] = [
    RUN,
    CLEAR,
    PREV_ITERATION,
    NEXT_ITERATION,
    PREV_CALL,
    NEXT_CALL,
    NOOP,
];
const DEFAULT_PYTHON: &str = "python3";

enum RunState {
    Running,
    Shown { view: View, warning: Option<String> },
    Failed(String),
}

struct Run {
    line: u32,
    state: RunState,
}

#[derive(Default)]
struct Document {
    text: String,
    run: Option<Run>,
}

impl Document {
    fn line_count(&self) -> u32 {
        self.text.split('\n').count() as u32
    }
}

struct Core {
    client: Client,
    documents: Mutex<HashMap<String, Document>>,
    config: OnceLock<Config>,
    workspace_root: OnceLock<Option<PathBuf>>,
}

pub struct Server {
    core: Arc<Core>,
}

impl Server {
    pub fn new(client: Client) -> Server {
        Server {
            core: Arc::new(Core {
                client,
                documents: Mutex::new(HashMap::new()),
                config: OnceLock::new(),
                workspace_root: OnceLock::new(),
            }),
        }
    }
}

fn arguments<T: DeserializeOwned>(arguments: Vec<Value>) -> Result<T> {
    serde_json::from_value(Value::Array(arguments))
        .map_err(|error| Error::invalid_params(error.to_string()))
}

fn action(title: &str, command: &str, arguments: Vec<Value>) -> CodeActionOrCommand {
    CodeActionOrCommand::CodeAction(CodeAction {
        title: title.to_string(),
        command: Some(Command {
            title: title.to_string(),
            command: command.to_string(),
            arguments: Some(arguments),
        }),
        ..Default::default()
    })
}

fn lens(line: u32, title: String, command: &str, arguments: Vec<Value>) -> CodeLens {
    CodeLens {
        range: Range::new(Position::new(line, 0), Position::new(line, 0)),
        command: Some(Command {
            title,
            command: command.to_string(),
            arguments: Some(arguments),
        }),
        data: None,
    }
}

fn view_command(uri: &str, action: Action) -> (&'static str, Vec<Value>) {
    match action {
        Action::Noop => (NOOP, vec![]),
        Action::Clear => (CLEAR, vec![json!(uri)]),
        Action::PrevCall => (PREV_CALL, vec![json!(uri)]),
        Action::NextCall => (NEXT_CALL, vec![json!(uri)]),
        Action::PrevIteration(line) => (PREV_ITERATION, vec![json!(uri), json!(line)]),
        Action::NextIteration(line) => (NEXT_ITERATION, vec![json!(uri), json!(line)]),
    }
}

fn warning(stop: Stop, config: &Config) -> String {
    match stop {
        Stop::Steps => format!("⚠ xray: stopped after {} steps", config.max_steps),
        Stop::Timeout => format!(
            "⚠ xray: stopped after {} s",
            config.timeout_ms as f64 / 1000.0
        ),
    }
}

fn render(uri: &str, document: &Document) -> Vec<CodeLens> {
    let Some(run) = &document.run else {
        return Vec::new();
    };
    let clear = |line| lens(line, "✕".to_string(), CLEAR, vec![json!(uri)]);
    match &run.state {
        RunState::Running => vec![
            lens(
                run.line,
                "⏳ xray: running…".to_string(),
                CLEAR,
                vec![json!(uri)],
            ),
            clear(run.line),
        ],
        RunState::Failed(message) => vec![lens(
            run.line,
            format!("⚠ xray: {message}"),
            CLEAR,
            vec![json!(uri)],
        )],
        RunState::Shown { view, warning } => {
            let mut lenses = Vec::new();
            if let (Some(text), Some(line)) = (warning, view.function_line()) {
                lenses.push(lens(line, text.clone(), CLEAR, vec![json!(uri)]));
            }
            for item in view.lenses(document.line_count()) {
                let (command, arguments) = view_command(uri, item.action);
                lenses.push(lens(item.line, item.title, command, arguments));
            }
            lenses
        }
    }
}

fn utf16_len(line: &str) -> u32 {
    line.encode_utf16().count() as u32
}

impl Core {
    fn config(&self) -> Config {
        self.config.get().cloned().unwrap_or_default()
    }

    fn python(&self) -> PathBuf {
        if let Some(python) = self.config().python {
            return python;
        }
        let candidates = [
            self.workspace_root
                .get()
                .cloned()
                .flatten()
                .map(|root| root.join(".venv/bin/python")),
            std::env::var_os("VIRTUAL_ENV").map(|venv| PathBuf::from(venv).join("bin/python")),
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|path| path.exists())
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PYTHON))
    }

    fn refresh(&self) {
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client.code_lens_refresh().await;
        });
    }

    fn start_run(self: &Arc<Self>, uri: &str, line: u32, name: Option<String>) {
        let mut documents = self.documents.lock().unwrap();
        let Some(document) = documents.get_mut(uri) else {
            return;
        };
        let source = document.text.clone();
        match &mut document.run {
            Some(run) if !matches!(run.state, RunState::Running) && name.is_some() => {
                run.line = line;
            }
            _ => {
                document.run = Some(Run {
                    line,
                    state: RunState::Running,
                })
            }
        }
        drop(documents);
        self.refresh();
        let Some(path) = uri
            .parse::<Uri>()
            .ok()
            .and_then(|uri| uri.to_file_path().map(|path| path.into_owned()))
        else {
            self.finish_run(
                uri,
                Outcome::Failed {
                    message: "only files on disk can be run".to_string(),
                },
            );
            return;
        };
        let request = TraceRequest {
            language: Language::Python,
            program: self.python(),
            path,
            source,
            line,
            name,
            limits: self.config().limits(),
        };
        let core = self.clone();
        let uri = uri.to_string();
        tokio::spawn(async move {
            let outcome = tracer::run(request).await;
            core.finish_run(&uri, outcome);
        });
    }

    fn finish_run(self: &Arc<Self>, uri: &str, outcome: Outcome) {
        let config = self.config();
        let mut documents = self.documents.lock().unwrap();
        let Some(document) = documents.get_mut(uri) else {
            return;
        };
        let Some(run) = &mut document.run else {
            return;
        };
        match outcome {
            Outcome::Traced(trace) => {
                let warning = trace.stopped.map(|stop| warning(stop, &config));
                run.state = match std::mem::replace(&mut run.state, RunState::Running) {
                    RunState::Shown { mut view, .. } => {
                        view.replace(trace);
                        RunState::Shown { view, warning }
                    }
                    _ => RunState::Shown {
                        view: View::new(trace),
                        warning,
                    },
                };
            }
            Outcome::Failed { message } => run.state = RunState::Failed(message),
            Outcome::NoCall { template } => {
                document.run = None;
                let edit_line = document
                    .text
                    .split('\n')
                    .nth(template.after_line as usize)
                    .map(utf16_len)
                    .unwrap_or(0);
                self.insert_template(uri, template, edit_line);
            }
            Outcome::NoFunction => {
                document.run = None;
                let client = self.client.clone();
                tokio::spawn(async move {
                    client
                        .show_message(
                            MessageType::INFO,
                            "xray: put the cursor inside a top-level function",
                        )
                        .await;
                });
            }
        }
        drop(documents);
        self.refresh();
    }

    fn insert_template(&self, uri: &str, template: Template, line_length: u32) {
        let Ok(parsed) = uri.parse::<Uri>() else {
            return;
        };
        let end = Position::new(template.after_line, line_length);
        let call_line = template.after_line + 3;
        let cursor = template.text.find('(').map_or(0, |index| index + 1) as u32;
        let edit = WorkspaceEdit {
            changes: Some(HashMap::from([(
                parsed.clone(),
                vec![TextEdit {
                    range: Range::new(end, end),
                    new_text: format!("\n\n\n{}", template.text),
                }],
            )])),
            ..Default::default()
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            if client.apply_edit(edit).await.is_err() {
                return;
            }
            let position = Position::new(call_line, cursor);
            let _ = client
                .show_document(ShowDocumentParams {
                    uri: parsed,
                    external: None,
                    take_focus: Some(true),
                    selection: Some(Range::new(position, position)),
                })
                .await;
        });
    }

    fn rerun(self: &Arc<Self>, uri: &str) {
        let documents = self.documents.lock().unwrap();
        let target = documents
            .get(uri)
            .and_then(|document| document.run.as_ref())
            .and_then(|run| match &run.state {
                RunState::Shown { view, .. } => Some((
                    view.function_line().unwrap_or(run.line),
                    Some(view.function_name().to_string()),
                )),
                RunState::Failed(_) => Some((run.line, None)),
                RunState::Running => None,
            });
        drop(documents);
        if let Some((line, name)) = target {
            self.start_run(uri, line, name);
        }
    }

    fn clear(&self, uri: &str) {
        if let Some(document) = self.documents.lock().unwrap().get_mut(uri) {
            document.run = None;
        }
        self.refresh();
    }

    fn act(&self, uri: &str, action: Action) {
        if let Some(Run {
            state: RunState::Shown { view, .. },
            ..
        }) = self
            .documents
            .lock()
            .unwrap()
            .get_mut(uri)
            .and_then(|document| document.run.as_mut())
        {
            view.act(action);
        }
        self.refresh();
    }
}

impl LanguageServer for Server {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let _ = self
            .core
            .config
            .set(Config::from_options(params.initialization_options));
        #[allow(deprecated)]
        let root = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .map(|folder| folder.uri.clone())
            .or(params.root_uri)
            .and_then(|uri| uri.to_file_path().map(|path| path.into_owned()));
        let _ = self.core.workspace_root.set(root);
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..Default::default()
                    },
                )),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: COMMANDS.map(String::from).to_vec(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        self.core
            .documents
            .lock()
            .unwrap()
            .entry(document.uri.to_string())
            .or_default()
            .text = document.text;
        self.core.refresh();
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        if let Some(document) = self.core.documents.lock().unwrap().get_mut(&uri) {
            for change in params.content_changes {
                text::apply_change(&mut document.text, change.range, &change.text);
                match (change.range, &mut document.run) {
                    (
                        Some(range),
                        Some(Run {
                            state: RunState::Shown { view, .. },
                            ..
                        }),
                    ) => view.apply_edit(
                        (range.start.line, range.start.character),
                        (range.end.line, range.end.character),
                        &change.text,
                    ),
                    (None, _) => document.run = None,
                    _ => {}
                }
            }
        }
        self.core.refresh();
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.core.rerun(&params.text_document.uri.to_string());
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri.to_string();
        let has_run = self
            .core
            .documents
            .lock()
            .unwrap()
            .get(&uri)
            .is_some_and(|document| document.run.is_some());
        let mut actions = vec![action(
            "xray: Run",
            RUN,
            vec![json!(uri), json!(params.range.start)],
        )];
        if has_run {
            actions.push(action("xray: Clear", CLEAR, vec![json!(uri)]));
        }
        Ok(Some(actions))
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri.to_string();
        let documents = self.core.documents.lock().unwrap();
        Ok(Some(
            documents
                .get(&uri)
                .map(|document| render(&uri, document))
                .unwrap_or_default(),
        ))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        let core = &self.core;
        match params.command.as_str() {
            RUN => {
                let (uri, position): (String, Position) = arguments(params.arguments)?;
                core.start_run(&uri, position.line, None);
            }
            CLEAR => {
                let (uri,): (String,) = arguments(params.arguments)?;
                core.clear(&uri);
            }
            PREV_ITERATION | NEXT_ITERATION => {
                let (uri, line): (String, u32) = arguments(params.arguments)?;
                let action = if params.command == PREV_ITERATION {
                    Action::PrevIteration(line)
                } else {
                    Action::NextIteration(line)
                };
                core.act(&uri, action);
            }
            PREV_CALL | NEXT_CALL => {
                let (uri,): (String,) = arguments(params.arguments)?;
                let action = if params.command == PREV_CALL {
                    Action::PrevCall
                } else {
                    Action::NextCall
                };
                core.act(&uri, action);
            }
            NOOP => {}
            other => return Err(Error::invalid_params(format!("unknown command {other}"))),
        }
        Ok(None)
    }
}
