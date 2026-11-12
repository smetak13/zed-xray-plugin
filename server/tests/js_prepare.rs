mod common;

use common::sample_path;
use xray::js::{Prepared, prepare};
use xray::trace::{Function, Template};

fn sample(name: &str) -> String {
    std::fs::read_to_string(sample_path(name)).unwrap()
}

fn ready_function(prepared: Prepared) -> Function {
    match prepared {
        Prepared::Ready { function, .. } => function,
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn finds_a_var_function_expression_like_leetcode() {
    let prepared = prepare(&sample("leetcode.js"), false, 3, None);

    assert_eq!(
        ready_function(prepared),
        Function {
            name: "twoSum".to_string(),
            line: 0,
            end_line: 10
        }
    );
}

#[test]
fn function_without_a_call_yields_a_template() {
    let prepared = prepare(&sample("no_call.js"), false, 1, None);

    assert_eq!(
        prepared,
        Prepared::NoCall(Template {
            after_line: 2,
            text: "add(/* a */, /* b */);".to_string()
        })
    );
}

#[test]
fn cursor_outside_a_function_yields_no_function() {
    let prepared = prepare("const x = 1;\n\nfunction f() {}\nf();\n", false, 0, None);

    assert_eq!(prepared, Prepared::NoFunction);
}

#[test]
fn falls_back_to_the_name_when_the_cursor_moved() {
    let prepared = prepare(&sample("no_call.js"), false, 3, Some("add"));

    assert!(matches!(prepared, Prepared::NoCall(_)));
}

#[test]
fn destructured_parameter_gets_a_positional_placeholder() {
    let prepared = prepare("function f({ a }, b) {}\n", false, 0, None);

    assert_eq!(
        prepared,
        Prepared::NoCall(Template {
            after_line: 0,
            text: "f(/* arg1 */, /* b */);".to_string()
        })
    );
}

#[test]
fn async_function_is_rejected() {
    let prepared = prepare("async function f() {}\nf();\n", false, 0, None);

    assert_eq!(
        prepared,
        Prepared::Failed("async functions and generators are not supported".to_string())
    );
}

#[test]
fn syntax_error_reports_the_parser_message_and_line() {
    let prepared = prepare("const a = 1;\nlet = = 2;\n", false, 1, None);

    match prepared {
        Prepared::Failed(message) => assert!(message.starts_with("line 2: "), "{message}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn typescript_annotations_parse() {
    let source = "function f(n: number): number {\n  return n;\n}\nf(1);\n";

    let prepared = prepare(source, true, 1, None);

    assert_eq!(ready_function(prepared).name, "f");
}

#[test]
fn exported_arrow_function_is_found() {
    let source = "export const sq = (x) => x * x;\nsq(3);\n";

    let prepared = prepare(source, false, 0, None);

    assert_eq!(ready_function(prepared).name, "sq");
}
