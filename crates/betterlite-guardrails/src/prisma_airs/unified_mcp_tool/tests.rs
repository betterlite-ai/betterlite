#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use rstest::rstest;

use crate::panw_prisma_airs::audit::Direction;
use crate::prisma_airs::request_data::RequestData;
use crate::prisma_airs::support::{Outcome, Run, execute, inputs, request_data};
use crate::prisma_airs_masking::mcp::McpServer;

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const ALLOW_MASKED: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1","prompt_masked_data":{"data":"{\"q\":\"MASKED\"}"}}"#;
const ALLOW_MASKED_TEXT: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1","prompt_masked_data":{"data":"MASKED"}}"#;
const ALLOW_MASKED_INVALID_JSON: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1","prompt_masked_data":{"data":"not json"}}"#;
const BLOCK_MASKED: &str = r#"{"action":"block","category":"pii","scan_id":"scan-2","prompt_masked_data":{"data":"{\"q\":\"MASKED\"}"}}"#;
const BLOCK_PLAIN: &str = r#"{"action":"block","category":"injection","scan_id":"scan-3"}"#;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn with_search() -> RequestData {
    let mut data = request_data();
    data.mcp_tool_name = Some("search".to_owned());
    data
}

fn detail_json(outcome: &Outcome) -> String {
    let error = outcome.result.as_ref().expect_err("blocked");
    serde_json::to_string(&error.detail().expect("detail")).expect("detail json")
}

#[tokio::test]
async fn an_mcp_invocation_sends_a_tool_event_with_its_arguments() {
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 1);
    assert!(outcome.bodies[0].contains(
        r#""tool_event":{"metadata":{"ecosystem":"mcp","method":"tools/call","server_name":"unknown","tool_invoked":"search"},"input":"{\"q\":\"x\"}"}"#
    ));
    assert_eq!(outcome.audit.scan_ids, ["scan-1"]);
}

#[tokio::test]
async fn the_request_server_name_wins_over_the_registry() {
    let mut run = Run::new(ALLOW);
    run.tool_names = BTreeMap::from([("search".to_owned(), "registry-server".to_owned())]);
    let mut data = with_search();
    data.mcp_server_name = Some("request-server".to_owned());
    data.mcp_arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""server_name":"request-server""#));
}

#[tokio::test]
async fn the_registry_resolves_a_server_by_id() {
    let mut run = Run::new(ALLOW);
    run.servers = BTreeMap::from([(
        "s1".to_owned(),
        McpServer {
            alias: Some("alias-a".to_owned()),
            server_name: Some("server-a".to_owned()),
            name: None,
            server_id: Some("s1".to_owned()),
        },
    )]);
    let mut data = with_search();
    data.server_id = Some("s1".to_owned());
    data.mcp_arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""server_name":"alias-a""#));
}

#[tokio::test]
async fn the_registry_resolves_a_server_by_tool_name() {
    let mut run = Run::new(ALLOW);
    run.tool_names = BTreeMap::from([("search".to_owned(), "server-b".to_owned())]);
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""server_name":"server-b""#));
}

#[tokio::test]
async fn the_legacy_server_name_wins_over_the_registry() {
    let mut run = Run::new(ALLOW);
    run.tool_names = BTreeMap::from([("search".to_owned(), "registry-server".to_owned())]);
    let mut data = with_search();
    data.server_name = Some("legacy-server".to_owned());
    data.mcp_arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""server_name":"legacy-server""#));
}

#[tokio::test]
async fn a_name_without_arguments_is_not_an_mcp_request() {
    let mut data = request_data();
    data.name = Some("search".to_owned());
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("nothing to scan");
    assert!(outcome.bodies.is_empty());
}

#[tokio::test]
async fn a_name_with_arguments_is_an_mcp_request() {
    let mut data = request_data();
    data.name = Some("search".to_owned());
    data.arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""tool_invoked":"search""#));
    assert!(outcome.bodies[0].contains(r#""input":"{\"q\":\"x\"}""#));
}

#[tokio::test]
async fn a_null_mcp_argument_falls_back_to_the_arguments_field() {
    let mut data = with_search();
    data.mcp_arguments = Some(GuardrailValue::Null);
    data.arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""input":"{\"q\":\"x\"}""#));
}

#[tokio::test]
async fn a_null_arguments_field_sends_no_input() {
    let mut data = with_search();
    data.arguments = Some(GuardrailValue::Null);
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""tool_invoked":"search""#));
    assert!(!outcome.bodies[0].contains(r#""input":"#));
}

#[rstest]
#[case::empty_string(Some(text("")))]
#[case::whitespace(Some(text("   ")))]
#[case::no_argument_field(None)]
#[tokio::test]
async fn a_tool_event_without_usable_arguments_has_no_input(
    #[case] arguments: Option<GuardrailValue>,
) {
    let mut data = with_search();
    data.mcp_arguments = arguments;
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""tool_invoked":"search""#));
    assert!(!outcome.bodies[0].contains(r#""input":"#));
}

#[tokio::test]
async fn an_empty_object_argument_is_still_sent() {
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[]));
    let outcome = execute(Run::new(ALLOW), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(r#""input":"{}""#));
}

#[tokio::test]
async fn an_allowed_scan_masks_structured_arguments_in_both_fields() {
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    data.arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(Run::new(ALLOW_MASKED), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    let expected = object(&[("q", text("MASKED"))]);
    assert_eq!(outcome.request_data.mcp_arguments, Some(expected.clone()));
    assert_eq!(outcome.request_data.arguments, Some(expected));
}

#[tokio::test]
async fn a_plain_text_argument_stays_text_when_masked() {
    let mut data = with_search();
    data.arguments = Some(text("secret"));
    let outcome = execute(Run::new(ALLOW_MASKED_TEXT), inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.request_data.arguments, Some(text("MASKED")));
    assert_eq!(outcome.request_data.mcp_arguments, None);
}

#[tokio::test]
async fn invalid_masked_json_blocks_structured_arguments() {
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(Run::new(ALLOW_MASKED_INVALID_JSON), inputs(&[]), data, None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocked").status(), 400);
    let detail = detail_json(&outcome);
    assert!(detail.contains("masked data is not valid JSON"));
    assert!(detail.contains(r#""type":"guardrail_violation""#));
    assert!(detail.contains(r#""code":"panw_prisma_airs_blocked""#));
    assert_eq!(
        outcome.request_data.mcp_arguments,
        Some(object(&[("q", text("secret"))]))
    );
}

#[tokio::test]
async fn a_masked_allow_without_an_argument_field_is_a_400() {
    let outcome = execute(Run::new(ALLOW_MASKED), inputs(&[]), with_search(), None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocked").status(), 400);
    let detail = detail_json(&outcome);
    assert!(detail.contains("no rewritable argument field present"));
    assert!(detail.contains(r#""type":"guardrail_violation""#));
    assert!(detail.contains(r#""code":"panw_prisma_airs_blocked""#));
}

#[tokio::test]
async fn a_blocked_mcp_request_with_masking_on_is_rewritten() {
    let run = Run {
        mask_request: true,
        ..Run::new(BLOCK_MASKED)
    };
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("masked instead of blocked");
    assert_eq!(
        outcome.request_data.mcp_arguments,
        Some(object(&[("q", text("MASKED"))]))
    );
}

#[tokio::test]
async fn a_blocked_mcp_request_without_masking_is_a_400() {
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(Run::new(BLOCK_PLAIN), inputs(&[]), data, None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocked").status(), 400);
    assert!(detail_json(&outcome).contains(r#""code":"panw_prisma_airs_blocked""#));
}

#[tokio::test]
async fn a_transient_mcp_failure_with_fallback_allow_leaves_the_arguments_alone() {
    let run = Run {
        status: StatusCode::SERVICE_UNAVAILABLE,
        fallback: FallbackAction::Allow,
        ..Run::new("{}")
    };
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("fallback allows");
    assert_eq!(
        outcome.request_data.mcp_arguments,
        Some(object(&[("q", text("secret"))]))
    );
    assert_eq!(outcome.audit.applied_guardrails, ["panw:unscanned", "panw"]);
    assert_eq!(outcome.audit.information.len(), 1);
}

#[tokio::test]
async fn a_transient_mcp_failure_without_fallback_is_a_500() {
    let run = Run {
        status: StatusCode::SERVICE_UNAVAILABLE,
        ..Run::new("{}")
    };
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocks").status(), 500);
}

#[tokio::test]
async fn masked_json_past_the_recursion_limit_blocks() {
    let masked = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let reply = format!(
        r#"{{"action":"allow","prompt_masked_data":{{"data":{}}}}}"#,
        serde_json::to_string(&masked).expect("encode")
    );
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("secret"))]));
    let outcome = execute(Run::new(&reply), inputs(&[]), data, None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocked").status(), 400);
    assert!(detail_json(&outcome).contains("not valid JSON"));
}

#[tokio::test]
async fn a_response_direction_never_scans_mcp() {
    let mut run = Run {
        direction: Direction::Response,
        ..Run::new(ALLOW)
    };
    run.tool_names = BTreeMap::from([("search".to_owned(), "server-b".to_owned())]);
    let mut data = with_search();
    data.mcp_arguments = Some(object(&[("q", text("x"))]));
    let outcome = execute(run, inputs(&[]), data, None).await;
    outcome.result.as_ref().expect("nothing to scan");
    assert!(outcome.bodies.is_empty());
}
