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

fn code_of(prepared: Prepared) -> String {
    match prepared {
        Prepared::Ready { code, .. } => code,
        other => panic!("expected Ready, got {other:?}"),
    }
}

fn analysis_of(prepared: Prepared) -> xray::js::Analysis {
    match prepared {
        Prepared::Ready { analysis, .. } => analysis,
        other => panic!("expected Ready, got {other:?}"),
    }
}

fn assert_contains(code: &str, part: &str) {
    assert!(code.contains(part), "missing {part:?} in:\n{code}");
}

#[test]
fn blanks_non_setup_statements_and_appends_the_call() {
    let source = "const K = 2;\nconsole.log(\"x\");\nfunction f() { return K }\nf();\n";

    let code = code_of(prepare(source, false, 2, None));

    let lines: Vec<&str> = code.split('\n').collect();
    assert_eq!(lines[0], "const K = 2;");
    assert!(lines[1].trim().is_empty(), "{:?}", lines[1]);
    assert!(lines[3].trim().is_empty(), "{:?}", lines[3]);
    assert!(code.ends_with("\nf();\n"), "{code}");
}

#[test]
fn probes_each_statement_with_variables_declared_before_it() {
    let source = "function f(a) {\n  let b = a;\n  b++;\n}\nf(1);\n";

    let code = code_of(prepare(source, false, 0, None));

    assert_contains(&code, "__xray.line(1, () => ({a})); let b = a;");
    assert_contains(&code, "__xray.line(2, () => ({a, b})); b++;");
}

#[test]
fn wraps_tests_of_if_while_and_for() {
    let source = "function f(n) {\n  if (n) {}\n  while (n) {}\n  for (let i = 0; i < n; i++) {}\n  for (;;) {}\n}\nf(1);\n";

    let code = code_of(prepare(source, false, 0, None));

    assert_contains(&code, "if ((__xray.line(1, () => ({n})), n))");
    assert_contains(&code, "while ((__xray.line(2, () => ({n})), n))");
    assert_contains(
        &code,
        "for (let i = 0; (__xray.line(3, () => ({n, i})), i < n); i++)",
    );
    assert_contains(&code, "for (;(__xray.line(4, () => ({n})), true);)");
}

#[test]
fn wraps_brace_less_bodies() {
    let source =
        "function f(xs) {\n  let s = 0;\n  for (const x of xs) s += x;\n  return s;\n}\nf([1]);\n";

    let code = code_of(prepare(source, false, 0, None));

    assert_contains(
        &code,
        "for (const x of xs) { __xray.line(2, () => ({xs, s, x})); __xray.line(2, () => ({xs, s, x})); s += x; }",
    );
}

#[test]
fn wraps_the_body_and_returns() {
    let source = "function f(a) {\n  if (a) return;\n  return a + 1;\n}\nf(1);\n";

    let code = code_of(prepare(source, false, 0, None));

    assert_contains(&code, "function f(a) {__xray.enter(() => ({a})); try {");
    assert_contains(&code, "return __xray.ret(undefined, () => ({a}));");
    assert_contains(&code, "return __xray.ret(a + 1, () => ({a}));");
    assert_contains(
        &code,
        "__xray.ret(undefined, () => ({a})); } catch (__xray_error) { __xray.raise(__xray_error); throw __xray_error } finally { __xray.exit() }}",
    );
}

#[test]
fn arrow_expression_body_becomes_a_block() {
    let source = "const sq = (x) => x * x;\nsq(3);\n";

    let code = code_of(prepare(source, false, 0, None));

    assert_contains(
        &code,
        "const sq = (x) => { __xray.enter(() => ({x})); try { __xray.line(0, () => ({x})); return __xray.ret(x * x, () => ({x})); } catch (__xray_error) { __xray.raise(__xray_error); throw __xray_error } finally { __xray.exit() } };",
    );
}

#[test]
fn does_not_instrument_nested_functions() {
    let source = "function f(xs) {\n  function g() { let z = 1; }\n  return xs.map((v) => { return v + 1; });\n}\nf([1]);\n";

    let code = code_of(prepare(source, false, 0, None));

    assert_contains(&code, "function g() { let z = 1; }");
    assert_contains(&code, "xs.map((v) => { return v + 1; })");
}

#[test]
fn analysis_matches_the_python_shape() {
    let analysis = analysis_of(prepare(&sample("bubble.js"), false, 1, None));

    let outer = &analysis.loops[&2];
    assert_eq!(
        (outer.line, outer.end_line, outer.body_start, outer.body_end),
        (2, 8, 3, 7)
    );
    assert_eq!(outer.targets, Some(vec!["i".to_string()]));
    let inner = &analysis.loops[&3];
    assert_eq!((inner.body_start, inner.body_end), (4, 6));
    assert_eq!(inner.targets, Some(vec!["j".to_string()]));
    assert_eq!(analysis.conditions[&4], (5, 5));
    assert!(analysis.assigned[&2].contains("i"));
    assert!(analysis.assigned[&1].contains("n"));
    assert!(!analysis.assigned.contains_key(&5));
}

#[test]
fn while_loops_have_no_targets_and_a_condition() {
    let source = "function f(n) {\n  while (n > 0) {\n    n -= 1;\n  }\n}\nf(2);\n";

    let analysis = analysis_of(prepare(source, false, 0, None));

    assert_eq!(analysis.loops[&1].targets, None);
    assert_eq!(analysis.conditions[&1], (2, 2));
    assert!(analysis.assigned[&2].contains("n"));
}
