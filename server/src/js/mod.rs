mod analysis;
mod edits;
mod instrument;
mod lines;
mod locate;

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

pub use analysis::{Analysis, LoopInfo};
use edits::Edits;
use instrument::Instrumenter;
use lines::LineIndex;
use locate::{Body, Target};

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
        let offset = error.labels.first().map_or(0, |label| label.offset());
        return Prepared::Failed(format!(
            "line {}: {}",
            lines.line(offset) + 1,
            error.message
        ));
    }
    let program = &parsed.program;
    let contains =
        |span: oxc_span::Span| lines.line(span.start) <= line && line <= lines.end_line(span.end);
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
    let Some(call) = locate::find_call(program, &target) else {
        return Prepared::NoCall(Template {
            after_line: function.end_line,
            text: locate::call_template(&target),
        });
    };
    let mut edits = Edits::default();
    for statement in &program.body {
        if !is_setup(statement) || statement.span() == call.span() {
            edits.blank(statement.span(), source);
        }
    }
    let analysis = instrument(source, &lines, &mut edits, &target);
    let call_text = &source[call.span().start as usize..call.span().end as usize];
    let mut code = edits.apply(source);
    if !code.ends_with('\n') {
        code.push('\n');
    }
    code.push_str(call_text);
    code.push('\n');
    Prepared::Ready {
        code,
        analysis,
        function,
    }
}

fn is_setup(statement: &Statement) -> bool {
    statement.as_declaration().is_some() || statement.as_module_declaration().is_some()
}

fn instrument(source: &str, lines: &LineIndex, edits: &mut Edits, target: &Target) -> Analysis {
    let mut instrumenter = Instrumenter::new(source, lines, edits);
    match target.body {
        Body::Function(function) => {
            if let Some(body) = &function.body {
                instrumenter.function(
                    &function.params,
                    body.span.start + 1,
                    body.span.end - 1,
                    &body.statements,
                );
            }
        }
        Body::Arrow(arrow) => match arrow.get_function_body() {
            Some(body) => instrumenter.function(
                &arrow.params,
                body.span.start + 1,
                body.span.end - 1,
                &body.statements,
            ),
            None => {
                let expression = arrow.get_expression().expect("an expression body");
                instrumenter.arrow_expression(&arrow.params, expression);
            }
        },
    }
    instrumenter.analysis
}
