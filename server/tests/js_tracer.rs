mod common;

use std::path::PathBuf;

use common::{event, lines, sample_path, trace_sample, trace_sample_with, traced};
use xray::trace::{Event, Outcome, Stop};
use xray::tracer::{self, Language, Limits, TraceRequest};

fn condition(line: u32, outcome: bool) -> Event {
    Event {
        condition: Some(outcome),
        ..event(line, &[])
    }
}

fn returned(line: u32, value: &str) -> Event {
    Event {
        returned: Some(value.to_string()),
        ..event(line, &[])
    }
}

#[tokio::test]
async fn js_records_changed_values_and_the_return_value_per_line() {
    let trace = traced(trace_sample("helpers.ts", 8).await);

    assert_eq!(
        trace.calls[0].scope.events,
        vec![
            event(8, &[("result", "[ 2, 14 ]")]),
            event(9, &[("same", "true")]),
            returned(10, "4")
        ]
    );
}

#[tokio::test]
async fn js_splits_loops_into_iterations_scoped_to_the_outer_iteration() {
    let trace = traced(trace_sample("nested_while.js", 0).await);

    let scope = &trace.calls[0].scope;
    assert_eq!(lines(scope), vec![1, 2, 9]);
    let outer = &scope.loops[0];
    assert_eq!(outer.line, 2);
    let labels: Vec<_> = outer.iterations.iter().map(|it| it.label.clone()).collect();
    assert_eq!(
        labels,
        vec![Some("i=0".into()), Some("i=1".into()), Some("i=2".into())]
    );
    let inner_counts: Vec<_> = outer
        .iterations
        .iter()
        .map(|it| it.loops.first().map_or(0, |run| run.iterations.len()))
        .collect();
    assert_eq!(inner_counts, vec![0, 1, 2]);
}

#[tokio::test]
async fn js_infers_conditions_and_records_in_place_mutation() {
    let trace = traced(trace_sample("bubble.js", 0).await);

    let outer = &trace.calls[0].scope.loops[0];
    let swapped = &outer.iterations[0].loops[0].iterations[0];
    let kept = &outer.iterations[1].loops[0].iterations[0];
    assert_eq!(
        swapped.events,
        vec![
            event(3, &[("j", "0")]),
            condition(4, true),
            event(5, &[("arr", "[ 1, 3, 2 ]")])
        ]
    );
    assert_eq!(
        kept.events,
        vec![event(3, &[("j", "0")]), condition(4, false)]
    );
}

#[tokio::test]
async fn js_while_header_is_true_inside_iterations_and_false_on_exit() {
    let trace = traced(trace_sample("nested_while.js", 0).await);

    let outer = &trace.calls[0].scope.loops[0].iterations[1];
    assert_eq!(outer.events.last(), Some(&condition(4, false)));
    assert_eq!(outer.loops[0].iterations[0].events[0], condition(4, true));
}

#[tokio::test]
async fn ts_recursion_has_calls_with_depth() {
    let trace = traced(trace_sample("fib.ts", 1).await);

    let depths: Vec<_> = trace.calls.iter().map(|call| call.depth).collect();
    assert_eq!(depths, vec![0, 1, 2, 2, 1]);
    let first = &trace.calls[0].scope.events;
    assert_eq!(first.last().unwrap().returned.as_deref(), Some("2"));
}

#[tokio::test]
async fn js_exception_is_recorded_once_at_the_raising_line() {
    let trace = traced(trace_sample("raises.js", 0).await);

    let raised = Event {
        exception: Some(
            "TypeError: Cannot read properties of undefined (reading 'toFixed')".to_string(),
        ),
        ..event(2, &[])
    };
    assert_eq!(
        trace.calls[0].scope.events,
        vec![event(1, &[("index", "2")]), raised]
    );
}

#[tokio::test]
async fn js_step_limit_stops_and_keeps_data() {
    let limits = Limits {
        max_steps: 50,
        ..Default::default()
    };

    let trace = traced(trace_sample_with("forever.js", 0, limits).await);

    assert_eq!(trace.stopped, Some(Stop::Steps));
    assert!(!trace.calls[0].scope.loops[0].iterations.is_empty());
}

#[tokio::test]
async fn js_timeout_stops() {
    let limits = Limits {
        timeout: std::time::Duration::from_millis(300),
        max_steps: u64::MAX,
    };

    let trace = traced(trace_sample_with("forever.js", 0, limits).await);

    assert_eq!(trace.stopped, Some(Stop::Timeout));
    assert!(!trace.calls[0].scope.loops[0].iterations.is_empty());
}

#[tokio::test]
async fn statements_on_one_line_merge() {
    let trace = traced(trace_sample("one_line.js", 0).await);

    assert_eq!(
        trace.calls[0].scope.events,
        vec![
            event(1, &[("a", "0"), ("b", "0")]),
            event(2, &[("a", "1"), ("b", "2")]),
            returned(3, "3")
        ]
    );
}

#[tokio::test]
async fn tdz_declaration_later_in_block_does_not_fail() {
    let trace = traced(trace_sample("one_line.js", 7).await);

    assert_eq!(
        trace.calls[0].scope.events,
        vec![event(7, &[]), returned(12, "1")]
    );
}

#[tokio::test]
async fn arrow_expression_body_records_return() {
    let trace = traced(trace_sample("one_line.js", 16).await);

    assert_eq!(trace.calls[0].scope.events, vec![returned(16, "9")]);
}

#[tokio::test]
async fn missing_node_reports_cannot_start() {
    let path = sample_path("bubble.js");
    let source = std::fs::read_to_string(&path).unwrap();

    let outcome = tracer::run(TraceRequest {
        language: Language::JavaScript,
        program: PathBuf::from("/nonexistent/node"),
        path,
        source,
        line: 0,
        name: None,
        limits: Limits::default(),
    })
    .await;

    match outcome {
        Outcome::Failed { message } => {
            assert!(
                message.starts_with("cannot start /nonexistent/node:"),
                "{message}"
            )
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_returned_sequence_keeps_the_function_result() {
    let trace = traced(trace_sample("edge_cases.js", 0).await);

    let events = &trace.calls[0].scope.events;
    assert_eq!(events.last().unwrap().returned.as_deref(), Some("3"));
}

#[tokio::test]
async fn a_function_declared_in_a_block_keeps_later_values() {
    let trace = traced(trace_sample("edge_cases.js", 5).await);

    let events = &trace.calls[0].scope.events;
    assert!(events.contains(&event(11, &[("a", "2")])), "{events:?}");
}

#[tokio::test]
async fn a_shadowing_let_keeps_values_earlier_in_the_block() {
    let trace = traced(trace_sample("edge_cases.js", 15).await);

    let events = &trace.calls[0].scope.events;
    assert_eq!(events[0], event(16, &[("y", "2")]));
    assert_eq!(events[1], event(18, &[("y", "4")]));
}

#[tokio::test]
async fn a_live_timer_in_the_file_does_not_block_the_result() {
    let trace = traced(trace_sample("timer.js", 2).await);

    assert_eq!(trace.calls[0].scope.events, vec![returned(3, "3")]);
}
