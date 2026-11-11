use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::trace::Outcome;

const TRACER: &str = include_str!("tracer.py");
const KILL_GRACE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct Limits {
    pub timeout: Duration,
    pub max_steps: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            timeout: Duration::from_millis(5000),
            max_steps: 100_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TraceRequest {
    pub python: PathBuf,
    pub path: PathBuf,
    pub source: String,
    pub line: u32,
    pub name: Option<String>,
    pub limits: Limits,
}

fn failed(message: impl Into<String>) -> Outcome {
    Outcome::Failed {
        message: message.into(),
    }
}

pub async fn run(request: TraceRequest) -> Outcome {
    let output = match tempfile::NamedTempFile::new() {
        Ok(file) => file,
        Err(error) => return failed(format!("cannot create a temp file: {error}")),
    };
    let spawned = Command::new(&request.python)
        .arg("-I")
        .arg("-c")
        .arg(TRACER)
        .arg(&request.path)
        .arg(request.line.to_string())
        .arg(request.name.as_deref().unwrap_or(""))
        .arg(output.path())
        .arg(request.limits.timeout.as_millis().to_string())
        .arg(request.limits.max_steps.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return failed(format!(
                "cannot start {}: {error}",
                request.python.display()
            ));
        }
    };
    let mut stdin = child.stdin.take().expect("piped stdin");
    let source = request.source.clone();
    tokio::spawn(async move {
        let _ = stdin.write_all(source.as_bytes()).await;
    });
    let mut stderr = child.stderr.take().expect("piped stderr");
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    let deadline = request.limits.timeout + KILL_GRACE;
    if tokio::time::timeout(deadline, child.wait()).await.is_err() {
        let _ = child.kill().await;
        return failed(format!(
            "stopped after {} s",
            request.limits.timeout.as_secs_f64()
        ));
    }
    let errors = errors.await.unwrap_or_default();
    match std::fs::read_to_string(output.path()) {
        Ok(json) if !json.is_empty() => serde_json::from_str(&json)
            .unwrap_or_else(|error| failed(format!("unreadable tracer output: {error}"))),
        _ => failed(
            errors
                .lines()
                .last()
                .unwrap_or("tracer exited without output"),
        ),
    }
}
