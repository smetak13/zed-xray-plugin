mod common;

use common::lsp::{LspClient, find, titles};
use common::sample_path;
use serde_json::json;

async fn opened(name: &str) -> LspClient {
    let mut client = LspClient::start(json!({})).await;
    let path = sample_path(name);
    let text = std::fs::read_to_string(&path).unwrap();
    client.open(&path, &text).await;
    client
}

#[tokio::test]
async fn advertises_every_command() {
    let client = LspClient::start(json!({})).await;

    let capabilities = &client.initialize_result["capabilities"];

    assert_eq!(capabilities["codeActionProvider"], json!(true));
    assert_eq!(
        capabilities["codeLensProvider"]["resolveProvider"],
        json!(false)
    );
    assert_eq!(
        capabilities["executeCommandProvider"]["commands"],
        json!([
            "xray.run",
            "xray.clear",
            "xray.prevIteration",
            "xray.nextIteration",
            "xray.prevCall",
            "xray.nextCall",
            "xray.noop"
        ])
    );
}

#[tokio::test]
async fn offers_run_and_after_a_run_also_clear() {
    let mut client = opened("helpers.py").await;

    let before: Vec<String> = client
        .code_actions(10)
        .await
        .into_iter()
        .map(|a| a.0)
        .collect();
    client.run_at(10).await;
    client.lenses_with(11, "result = [2, 14]").await;
    let after: Vec<String> = client
        .code_actions(10)
        .await
        .into_iter()
        .map(|a| a.0)
        .collect();

    assert_eq!(before, vec!["xray: Run"]);
    assert_eq!(after, vec!["xray: Run", "xray: Clear"]);
}

#[tokio::test]
async fn running_shows_values_below_the_lines() {
    let mut client = opened("helpers.py").await;

    client.run_at(10).await;

    let lenses = client.lenses_with(13, "↩ 4").await;
    assert_eq!(
        titles(&lenses),
        vec![
            (9, "✕".to_string()),
            (11, "result = [2, 14]".to_string()),
            (12, "root = 4".to_string()),
            (13, "↩ 4".to_string())
        ]
    );
    client.shutdown().await;
}

#[tokio::test]
async fn clicking_the_arrows_moves_the_loop() {
    let mut client = opened("nested_while.py").await;
    client.run_at(0).await;
    let lenses = client.lenses_with(2, "i=0 · 1/3").await;

    client.click(find(&lenses, 2, "▶")).await;

    client.lenses_with(2, "i=1 · 2/3").await;
}

#[tokio::test]
async fn clear_removes_every_lens() {
    let mut client = opened("helpers.py").await;
    client.run_at(10).await;
    let lenses = client.lenses_with(9, "✕").await;

    client.click(find(&lenses, 9, "✕")).await;

    client.lenses_until(|lenses| lenses.is_empty()).await;
}

#[tokio::test]
async fn a_function_without_a_call_gets_a_call_template_inserted() {
    let mut client = opened("no_call.py").await;

    client.run_at(1).await;

    let edits = client.received_until("workspace/applyEdit").await;
    let uri = client.uri.clone();
    assert_eq!(
        edits[0]["edit"]["changes"][uri.as_str()][0],
        json!({
            "range": {
                "start": { "line": 1, "character": 16 },
                "end": { "line": 1, "character": 16 }
            },
            "newText": "\n\n\nadd(a=, b=)"
        })
    );
}

#[tokio::test]
async fn saving_runs_the_function_again() {
    let mut client = opened("nested_while.py").await;
    client.run_at(0).await;
    client.lenses_with(4, "j = 0").await;

    client.change((3, 12, 3, 13), "5").await;
    client.save().await;

    client.lenses_with(4, "j = 5").await;
}

#[tokio::test]
async fn a_failed_run_shows_a_warning_that_clears_on_click() {
    let mut client = LspClient::start(json!({ "python": "/nonexistent/python" })).await;
    let path = sample_path("helpers.py");
    client
        .open(&path, &std::fs::read_to_string(&path).unwrap())
        .await;

    client.run_at(10).await;

    let lenses = client
        .lenses_until(|lenses| {
            lenses
                .iter()
                .any(|lens| lens.title.starts_with("⚠ xray: cannot start"))
        })
        .await;
    assert_eq!(lenses[0].command, "xray.clear");
}

#[tokio::test]
async fn hitting_the_step_limit_shows_a_warning_above_the_def() {
    let mut client = LspClient::start(json!({ "maxSteps": 20 })).await;
    let path = sample_path("forever.py");
    client
        .open(&path, &std::fs::read_to_string(&path).unwrap())
        .await;

    client.run_at(1).await;

    client
        .lenses_with(0, "⚠ xray: stopped after 20 steps")
        .await;
}

#[tokio::test]
async fn running_a_javascript_file_shows_values_below_the_lines() {
    let mut client = opened("bubble.js").await;

    client.run_at(0).await;

    client.lenses_with(2, "n = 3").await;
    client.shutdown().await;
}

#[tokio::test]
async fn a_typescript_function_without_a_call_gets_a_template_inserted() {
    let mut client = opened("no_call.ts").await;

    client.run_at(1).await;

    let edits = client.received_until("workspace/applyEdit").await;
    let uri = client.uri.clone();
    assert_eq!(
        edits[0]["edit"]["changes"][uri.as_str()][0]["newText"],
        json!("\n\n\nadd(/* a */, /* b */);")
    );
}

#[tokio::test]
async fn a_missing_node_shows_a_warning() {
    let mut client = LspClient::start(json!({ "node": "/nonexistent/node" })).await;
    let path = sample_path("bubble.js");
    client
        .open(&path, &std::fs::read_to_string(&path).unwrap())
        .await;

    client.run_at(0).await;

    client
        .lenses_until(|lenses| {
            lenses.iter().any(|lens| {
                lens.title
                    .starts_with("⚠ xray: cannot start /nonexistent/node")
            })
        })
        .await;
}
