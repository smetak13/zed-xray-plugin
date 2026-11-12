mod analysis;
mod lines;
mod locate;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub use analysis::{Analysis, LoopInfo};
use lines::LineIndex;

use crate::trace::{Function, Template};

const ASYNC_UNSUPPORTED: &str = "async functions and generators are not supported";

#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    Ready {
        code: String,
        analysis: Analysis,
        function: Function,
    },
    NoFunction,
    NoCall(Template),
    Failed(String),
}

pub fn prepare(source: &str, typescript: bool, line: u32, name: Option<&str>) -> Prepared {
    let allocator = Allocator::default();
    let source_type = if typescript {
        SourceType::ts()
    } else {
        SourceType::mjs()
    }
    .with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let lines = LineIndex::new(source);
    if let Some(error) = parsed.diagnostics.first() {
        let offset = error.labels.first().map_or(0, |label| label.offset() as u32);
        return Prepared::Failed(format!("line {}: {}", lines.line(offset) + 1, error.message));
    }
    let program = &parsed.program;
    let contains = |span: oxc_span::Span| {
        lines.line(span.start) <= line && line <= lines.end_line(span.end)
    };
    let Some(target) = locate::find_function(program, contains, name) else {
        return Prepared::NoFunction;
    };
    if target.is_async_or_generator() {
        return Prepared::Failed(ASYNC_UNSUPPORTED.to_string());
    }
    let function = Function {
        name: target.name.clone(),
        line: lines.line(target.statement.start),
        end_line: lines.end_line(target.statement.end),
    };
    if locate::find_call(program, &target).is_none() {
        return Prepared::NoCall(Template {
            after_line: function.end_line,
            text: locate::call_template(&target),
        });
    }
    Prepared::Ready {
        code: source.to_string(),
        analysis: Analysis::default(),
        function,
    }
}
