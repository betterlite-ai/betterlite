#![cfg(test)]

use crate::panw_prisma_airs::audit::Direction;
use crate::prisma_airs::inputs::Inputs;
use crate::prisma_airs::support::{
    Outcome, Run, execute, inputs, prompt_json, request_data, response_json, tool_calls,
};

const WEATHER: &str = r#"[{"function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]"#;
const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const ALLOW_MASKED: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1","prompt_masked_data":{"data":"get_weather\n{\"city\":\"XXXX\"}"}}"#;
const BLOCK_MASKED: &str = r#"{"action":"block","category":"pii","scan_id":"scan-2","prompt_masked_data":{"data":"get_weather\n{\"city\":\"XXXX\"}"}}"#;
const BLOCK_PLAIN: &str = r#"{"action":"block","category":"injection","scan_id":"scan-3"}"#;

fn with_tool_calls(json: &str) -> Inputs {
    let mut given = inputs(&[]);
    given.tool_calls = Some(tool_calls(json));
    given
}

fn arguments(outcome: &Outcome, index: usize) -> Option<&str> {
    outcome
        .inputs
        .tool_calls
        .as_ref()?
        .get(index)?
        .function
        .as_ref()?
        .arguments
        .as_deref()
}

#[tokio::test]
async fn each_tool_call_is_scanned_with_its_name_and_arguments() {
    let outcome = execute(
        Run::new(ALLOW),
        with_tool_calls(WEATHER),
        request_data(),
        None,
    )
    .await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 1);
    assert!(outcome.bodies[0].contains(&prompt_json("get_weather\n{\"city\":\"Paris\"}")));
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"Paris"}"#));
    assert_eq!(outcome.audit.scan_ids, ["scan-1"]);
}

#[tokio::test]
async fn an_allowed_scan_writes_the_masked_arguments_back() {
    let outcome = execute(
        Run::new(ALLOW_MASKED),
        with_tool_calls(WEATHER),
        request_data(),
        None,
    )
    .await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"XXXX"}"#));
}

#[tokio::test]
async fn a_blocked_call_with_masking_on_is_rewritten() {
    let run = Run {
        mask_request: true,
        ..Run::new(BLOCK_MASKED)
    };
    let outcome = execute(run, with_tool_calls(WEATHER), request_data(), None).await;
    outcome.result.as_ref().expect("masked instead of blocked");
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"XXXX"}"#));
}

#[tokio::test]
async fn a_blocked_call_without_masking_is_a_400() {
    let outcome = execute(
        Run::new(BLOCK_PLAIN),
        with_tool_calls(WEATHER),
        request_data(),
        None,
    )
    .await;
    assert_eq!(outcome.result.as_ref().expect_err("blocked").status(), 400);
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"Paris"}"#));
}

#[tokio::test]
async fn a_response_tool_call_is_scanned_as_response_text() {
    let run = Run {
        direction: Direction::Response,
        ..Run::new(ALLOW)
    };
    let outcome = execute(run, with_tool_calls(WEATHER), request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(&response_json("get_weather\n{\"city\":\"Paris\"}")));
}
