mod common;

use common::{document_lines, titles, trace_sample, traced, view_of};
use xray::view::Action;

fn pairs(items: &[(u32, &str)]) -> Vec<(u32, String)> {
    items
        .iter()
        .map(|(line, title)| (*line, title.to_string()))
        .collect()
}

#[tokio::test]
async fn shows_values_below_each_line_and_a_clear_button_above_the_def() {
    let view = view_of("helpers.py", 9).await;

    let lenses = view.lenses(document_lines("helpers.py"));

    assert_eq!(
        titles(&lenses),
        pairs(&[
            (9, "✕"),
            (11, "result = [2, 14]"),
            (12, "root = 4"),
            (13, "↩ 4")
        ])
    );
    assert_eq!(lenses[0].action, Action::Clear);
    assert_eq!(lenses[1].action, Action::Noop);
}

#[tokio::test]
async fn starts_on_the_first_iteration_with_navigation_above_the_loop_header() {
    let view = view_of("nested_while.py", 0).await;

    let lenses = view.lenses(document_lines("nested_while.py"));

    assert_eq!(
        titles(&lenses),
        pairs(&[
            (0, "✕"),
            (2, "total = 0"),
            (2, "◀"),
            (2, "i=0 · 1/3"),
            (2, "▶"),
            (3, "i = 0"),
            (4, "j = 0"),
            (5, "→ False"),
            (8, "↩ 1")
        ])
    );
    assert_eq!(lenses[2].action, Action::PrevIteration(2));
    assert_eq!(lenses[4].action, Action::NextIteration(2));
}

#[tokio::test]
async fn moving_a_loop_shows_its_iteration_and_stops_at_the_last_one() {
    let mut view = view_of("nested_while.py", 0).await;

    for _ in 0..3 {
        view.act(Action::NextIteration(2));
    }

    assert_eq!(
        titles(&view.lenses(document_lines("nested_while.py"))),
        pairs(&[
            (0, "✕"),
            (2, "total = 0"),
            (2, "◀"),
            (2, "i=2 · 3/3"),
            (2, "▶"),
            (3, "i = 2"),
            (4, "j = 0"),
            (4, "◀"),
            (4, "1/2"),
            (4, "▶"),
            (5, "→ True"),
            (6, "total = 0"),
            (7, "j = 1"),
            (8, "↩ 1")
        ])
    );
}

#[tokio::test]
async fn moving_the_outer_loop_resets_the_inner_loop_to_its_first_iteration() {
    let mut view = view_of("nested_while.py", 0).await;
    view.act(Action::NextIteration(2));
    view.act(Action::NextIteration(2));
    view.act(Action::NextIteration(4));

    view.act(Action::PrevIteration(2));
    view.act(Action::NextIteration(2));

    let lenses = titles(&view.lenses(document_lines("nested_while.py")));
    assert!(lenses.contains(&(4, "1/2".to_string())), "{lenses:?}");
}

#[tokio::test]
async fn navigates_recursive_calls_above_the_def() {
    let mut view = view_of("fib.py", 1).await;

    view.act(Action::NextCall);

    let lenses = view.lenses(document_lines("fib.py"));
    assert_eq!(
        titles(&lenses),
        pairs(&[
            (0, "◀"),
            (0, "call 2/5 · depth 1"),
            (0, "▶"),
            (0, "✕"),
            (2, "→ False"),
            (4, "↩ 1")
        ])
    );
    assert_eq!(lenses[0].action, Action::PrevCall);
    assert_eq!(lenses[2].action, Action::NextCall);
}

#[tokio::test]
async fn inserting_a_line_shifts_the_lenses_below_it() {
    let mut view = view_of("nested_while.py", 0).await;

    view.apply_edit((1, 0), (1, 0), "    x = 1\n");

    let lines: Vec<u32> = view
        .lenses(document_lines("nested_while.py") + 1)
        .iter()
        .map(|lens| lens.line)
        .collect();
    assert_eq!(lines, vec![0, 3, 3, 3, 3, 4, 5, 6, 9]);
}

#[tokio::test]
async fn editing_a_line_hides_its_values() {
    let mut view = view_of("nested_while.py", 0).await;

    view.apply_edit((3, 12), (3, 13), "5");

    let lenses = titles(&view.lenses(document_lines("nested_while.py")));
    assert!(!lenses.contains(&(4, "j = 0".to_string())), "{lenses:?}");
    assert!(lenses.contains(&(5, "→ False".to_string())), "{lenses:?}");
}

#[tokio::test]
async fn a_new_trace_keeps_the_selected_iterations_and_forgets_old_edits() {
    let mut view = view_of("nested_while.py", 0).await;
    view.act(Action::NextIteration(2));
    view.apply_edit((3, 12), (3, 13), "5");

    view.replace(traced(trace_sample("nested_while.py", 0).await));

    let lenses = titles(&view.lenses(document_lines("nested_while.py")));
    assert!(lenses.contains(&(2, "i=1 · 2/3".to_string())), "{lenses:?}");
    assert!(lenses.contains(&(4, "j = 0".to_string())), "{lenses:?}");
}
