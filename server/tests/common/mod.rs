#![allow(dead_code)]

use std::path::PathBuf;

use xray::trace::Outcome;
use xray::tracer::{self, Language, Limits, TraceRequest};

pub fn sample_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/samples")
        .join(name)
}

pub async fn trace_sample(name: &str, line: u32) -> Outcome {
    trace_sample_with(name, line, Limits::default()).await
}

pub async fn trace_sample_with(name: &str, line: u32, limits: Limits) -> Outcome {
    let path = sample_path(name);
    let source = std::fs::read_to_string(&path).unwrap();
    let (language, program) = match path.extension().and_then(|extension| extension.to_str()) {
        Some("js") => (Language::JavaScript, "node"),
        Some("ts") => (Language::TypeScript, "node"),
        _ => (Language::Python, "python3"),
    };
    tracer::run(TraceRequest {
        language,
        program: PathBuf::from(program),
        path,
        source,
        line,
        name: None,
        limits,
    })
    .await
}

pub fn traced(outcome: Outcome) -> xray::trace::Trace {
    match outcome {
        Outcome::Traced(trace) => trace,
        other => panic!("expected a trace, got {other:?}"),
    }
}

pub fn event(line: u32, values: &[(&str, &str)]) -> xray::trace::Event {
    xray::trace::Event {
        line,
        values: values
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        ..Default::default()
    }
}

pub fn lines(scope: &xray::trace::Scope) -> Vec<u32> {
    scope.events.iter().map(|event| event.line).collect()
}

pub async fn view_of(name: &str, line: u32) -> xray::view::View {
    xray::view::View::new(traced(trace_sample(name, line).await))
}

pub fn document_lines(name: &str) -> u32 {
    std::fs::read_to_string(sample_path(name))
        .unwrap()
        .lines()
        .count() as u32
}

pub fn titles(lenses: &[xray::view::Lens]) -> Vec<(u32, String)> {
    lenses
        .iter()
        .map(|lens| (lens.line, lens.title.clone()))
        .collect()
}
pub mod lsp;
