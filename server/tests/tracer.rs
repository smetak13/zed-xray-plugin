mod common;

use common::{event, lines, trace_sample, trace_sample_with, traced};
use xray::trace::{Outcome, Template};

#[tokio::test]
async fn function_without_a_call_yields_a_call_template() {
    let outcome = trace_sample("no_call.py", 1).await;

    assert_eq!(
        outcome,
        Outcome::NoCall {
            template: Template {
                after_line: 1,
                text: "add(a=, b=)".to_string()
            }
        }
    );
}

#[tokio::test]
async fn records_changed_values_and_the_return_value_per_line() {
    let trace = traced(trace_sample("helpers.py", 9).await);

    let returned = xray::trace::Event {
        returned: Some("4".to_string()),
        ..event(12, &[])
    };
    assert_eq!(
        trace.calls[0].scope.events,
        vec![
            event(10, &[("result", "[2, 14]")]),
            event(11, &[("root", "4")]),
            returned
        ]
    );
}

#[tokio::test]
async fn splits_loops_into_iterations_scoped_to_the_outer_iteration() {
    let trace = traced(trace_sample("nested_while.py", 0).await);

    let scope = &trace.calls[0].scope;
    assert_eq!(lines(scope), vec![1, 2, 7]);
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
    assert_eq!(lines(&outer.iterations[2]), vec![2, 3, 4]);
    assert_eq!(
        lines(&outer.iterations[2].loops[0].iterations[1]),
        vec![4, 5, 6]
    );
}

#[tokio::test]
async fn shows_an_assigned_name_even_when_its_value_did_not_change() {
    let trace = traced(trace_sample("nested_while.py", 0).await);

    let inner = &trace.calls[0].scope.loops[0].iterations[1].loops[0].iterations[0];
    assert_eq!(inner.events[1], event(5, &[("total", "0")]));
}

fn condition(line: u32, outcome: bool) -> xray::trace::Event {
    xray::trace::Event {
        condition: Some(outcome),
        ..event(line, &[])
    }
}

#[tokio::test]
async fn infers_if_outcomes_and_records_in_place_mutation() {
    let trace = traced(trace_sample("bubble.py", 0).await);

    let outer = &trace.calls[0].scope.loops[0];
    let swapped = &outer.iterations[0].loops[0].iterations[0];
    let kept = &outer.iterations[1].loops[0].iterations[0];
    assert_eq!(
        swapped.events,
        vec![
            event(3, &[("j", "0")]),
            condition(4, true),
            event(5, &[("arr", "[1, 3, 2]")])
        ]
    );
    assert_eq!(
        kept.events,
        vec![event(3, &[("j", "0")]), condition(4, false)]
    );
}

#[tokio::test]
async fn while_header_is_true_inside_iterations_and_false_on_exit() {
    let trace = traced(trace_sample("nested_while.py", 0).await);

    let outer = &trace.calls[0].scope.loops[0].iterations[1];
    assert_eq!(outer.events.last(), Some(&condition(4, false)));
    assert_eq!(outer.loops[0].iterations[0].events[0], condition(4, true));
}

#[tokio::test]
async fn records_every_recursive_call_with_its_depth() {
    let trace = traced(trace_sample("fib.py", 1).await);

    let depths: Vec<_> = trace.calls.iter().map(|call| call.depth).collect();
    assert_eq!(depths, vec![0, 1, 2, 2, 1]);
    let first = &trace.calls[0].scope.events;
    assert_eq!(first.last().unwrap().returned.as_deref(), Some("2"));
}

#[tokio::test]
async fn records_an_exception_on_the_raising_line_without_a_return_value() {
    let trace = traced(trace_sample("raises.py", 0).await);

    let raised = xray::trace::Event {
        exception: Some("IndexError: list index out of range".to_string()),
        ..event(2, &[])
    };
    assert_eq!(
        trace.calls[0].scope.events,
        vec![event(1, &[("index", "2")]), raised]
    );
}

#[tokio::test]
async fn stops_after_the_step_limit_and_keeps_what_was_recorded() {
    let limits = xray::tracer::Limits {
        max_steps: 50,
        ..Default::default()
    };

    let trace = traced(trace_sample_with("forever.py", 0, limits).await);

    assert_eq!(trace.stopped, Some(xray::trace::Stop::Steps));
    assert!(!trace.calls[0].scope.loops[0].iterations.is_empty());
}

#[tokio::test]
async fn stops_after_the_timeout_and_keeps_what_was_recorded() {
    let limits = xray::tracer::Limits {
        timeout: std::time::Duration::from_millis(300),
        max_steps: u64::MAX,
    };

    let trace = traced(trace_sample_with("forever.py", 0, limits).await);

    assert_eq!(trace.stopped, Some(xray::trace::Stop::Timeout));
    assert!(!trace.calls[0].scope.loops[0].iterations.is_empty());
}

#[tokio::test]
async fn traces_the_wrapped_function_of_a_decorated_one() {
    let trace = traced(trace_sample("decorated.py", 5).await);

    assert_eq!(
        trace.calls[0].scope.events[0],
        event(5, &[("result", "16")])
    );
}
