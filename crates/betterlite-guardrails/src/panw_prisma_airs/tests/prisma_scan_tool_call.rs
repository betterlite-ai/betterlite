use std::time::{Duration, SystemTime};

use axum::http::StatusCode;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use rstest::rstest;

use super::support::{guardrail, metadata, serve, settings};
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::error_detail::{ErrorCode, GuardrailHttpError};
use crate::panw_prisma_airs::tool_scan::scan_tool_calls_for_guardrail;
use crate::panw_prisma_airs::tool_scan::{ToolCall, ToolScan};

const WEATHER: &str = r#"[{"function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]"#;
const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const ALLOW_MASKED: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1","prompt_masked_data":{"data":"get_weather\n{\"city\":\"XXXX\"}"}}"#;
const BLOCK_MASKED: &str = r#"{"action":"block","category":"pii","scan_id":"scan-2","prompt_masked_data":{"data":"get_weather\n{\"city\":\"XXXX\"}"},"response_masked_data":{"data":"get_weather\n{\"city\":\"YYYY\"}"}}"#;
const BLOCK_PLAIN: &str = r#"{"action":"block","category":"injection","scan_id":"scan-3"}"#;

struct Outcome {
    result: Result<(), GuardrailHttpError>,
    calls: Vec<ToolCall>,
    audit: GuardrailAudit,
    bodies: Vec<String>,
}

struct Setup {
    status: StatusCode,
    reply: &'static str,
    direction: Direction,
    mask_request: bool,
    mask_response: bool,
    fallback: FallbackAction,
    call_id: Option<&'static str>,
}

impl Setup {
    fn new(reply: &'static str) -> Self {
        Self {
            status: StatusCode::OK,
            reply,
            direction: Direction::Request,
            mask_request: false,
            mask_response: false,
            fallback: FallbackAction::Block,
            call_id: Some("call-1"),
        }
    }
}

async fn run(setup: Setup, calls_json: &str) -> Outcome {
    let server = serve(setup.status, setup.reply.as_bytes()).await;
    let mut config = settings(&server.base);
    config.masking.content.request_content = setup.mask_request;
    config.masking.content.response_content = setup.mask_response;
    config.fallback_on_error = setup.fallback;
    let guardrail = guardrail(config);
    let mut calls: Vec<ToolCall> = serde_json::from_str(calls_json).expect("tool calls parse");
    let mut audit = GuardrailAudit::default();
    let scan_metadata = metadata();
    let scan = ToolScan {
        direction: setup.direction,
        metadata: &scan_metadata,
        call_id: setup.call_id,
        start_time: SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(2))
            .expect("time in range"),
    };
    let result = scan_tool_calls_for_guardrail(&guardrail, &mut calls, &scan, &mut audit).await;
    let bodies = server
        .captured
        .lock()
        .await
        .iter()
        .map(|request| request.body.clone())
        .collect();
    Outcome {
        result,
        calls,
        audit,
        bodies,
    }
}

fn arguments(outcome: &Outcome, index: usize) -> Option<&str> {
    outcome.calls[index].function.as_ref()?.arguments.as_deref()
}

fn prompt_json(text: &str) -> String {
    let encoded = serde_json::to_string(text).expect("text encodes");
    format!(r#""contents":[{{"prompt":{encoded}}}]"#)
}

#[tokio::test]
async fn the_name_and_arguments_are_scanned_as_one_prompt() {
    let outcome = run(Setup::new(ALLOW), WEATHER).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 1);
    assert!(outcome.bodies[0].contains(&prompt_json("get_weather\n{\"city\":\"Paris\"}")));
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"Paris"}"#));
    assert_eq!(outcome.audit.scan_ids, ["scan-1"]);
}

#[tokio::test]
async fn a_tool_call_in_a_response_is_scanned_as_response_text() {
    let setup = Setup {
        direction: Direction::Response,
        ..Setup::new(ALLOW)
    };
    let outcome = run(setup, WEATHER).await;
    outcome.result.as_ref().expect("allowed");
    assert!(
        outcome.bodies[0]
            .contains(r#""contents":[{"response":"get_weather\n{\"city\":\"Paris\"}"}]"#)
    );
    assert!(outcome.bodies[0].contains(r#""is_response":true"#));
}

#[tokio::test]
async fn an_allowed_scan_applies_the_masked_arguments_whatever_the_mask_flags_say() {
    let outcome = run(Setup::new(ALLOW_MASKED), WEATHER).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"XXXX"}"#));
}

#[rstest]
#[case::request(Direction::Request, true, false, r#"{"city":"XXXX"}"#)]
#[case::response(Direction::Response, false, true, r#"{"city":"YYYY"}"#)]
#[tokio::test]
async fn a_blocked_scan_is_masked_when_its_side_has_masking_on(
    #[case] direction: Direction,
    #[case] mask_request: bool,
    #[case] mask_response: bool,
    #[case] expected: &str,
) {
    let setup = Setup {
        direction,
        mask_request,
        mask_response,
        ..Setup::new(BLOCK_MASKED)
    };
    let outcome = run(setup, WEATHER).await;
    outcome.result.as_ref().expect("masked instead of blocked");
    assert_eq!(arguments(&outcome, 0), Some(expected));
}

#[rstest]
#[case::flags_off(Direction::Request, false, false)]
#[case::only_the_response_flag_on(Direction::Request, false, true)]
#[case::only_the_request_flag_on(Direction::Response, true, false)]
#[tokio::test]
async fn a_blocked_scan_without_masking_for_its_side_is_a_400(
    #[case] direction: Direction,
    #[case] mask_request: bool,
    #[case] mask_response: bool,
) {
    let setup = Setup {
        direction,
        mask_request,
        mask_response,
        ..Setup::new(BLOCK_MASKED)
    };
    let outcome = run(setup, WEATHER).await;
    let error = outcome.result.as_ref().expect_err("blocked");
    assert_eq!(error.status, 400);
    assert_eq!(error.detail.error.category, "pii");
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"Paris"}"#));
}

#[tokio::test]
async fn a_blocked_request_keeps_the_caller_masked_prompt_in_the_error_detail() {
    let outcome = run(Setup::new(BLOCK_MASKED), WEATHER).await;
    let error = outcome.result.as_ref().expect_err("blocked");
    let json = serde_json::to_string(&error.detail).expect("detail");
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsBlocked);
    assert!(json.contains("prompt_masked_data"));
    assert!(!json.contains("response_masked_data"));
}

#[tokio::test]
async fn a_blocked_response_uses_the_response_code() {
    let setup = Setup {
        direction: Direction::Response,
        ..Setup::new(BLOCK_PLAIN)
    };
    let error = run(setup, WEATHER).await.result.expect_err("blocked");
    assert_eq!(error.status, 400);
    assert_eq!(
        error.detail.error.code,
        ErrorCode::PanwPrismaAirsResponseBlocked
    );
}

#[tokio::test]
async fn a_blocked_scan_without_masked_data_is_a_400_even_with_masking_on() {
    let setup = Setup {
        mask_request: true,
        ..Setup::new(BLOCK_PLAIN)
    };
    let error = run(setup, WEATHER).await.result.expect_err("blocked");
    assert_eq!(error.status, 400);
    assert_eq!(error.detail.error.category, "injection");
}

#[tokio::test]
async fn a_name_only_call_is_scanned_and_cannot_be_masked() {
    let calls = r#"[{"function":{"name":"get_weather","arguments":"  "}}]"#;
    let outcome = run(Setup::new(ALLOW_MASKED), calls).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(&prompt_json("get_weather")));
    assert_eq!(arguments(&outcome, 0), Some("  "));
}

#[tokio::test]
async fn an_arguments_only_call_takes_the_whole_masked_text() {
    let reply = r#"{"action":"allow","prompt_masked_data":{"data":"{\"city\":\"XXXX\"}"}}"#;
    let calls = r#"[{"function":{"arguments":"{\"city\":\"Paris\"}"}}]"#;
    let outcome = run(Setup::new(reply), calls).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(&prompt_json("{\"city\":\"Paris\"}")));
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"XXXX"}"#));
}

#[tokio::test]
async fn masked_text_without_a_newline_cannot_be_applied() {
    let reply = r#"{"action":"allow","prompt_masked_data":{"data":"get_weather"}}"#;
    let outcome = run(Setup::new(reply), WEATHER).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"Paris"}"#));
}

#[rstest]
#[case::no_function(r#"[{"id":"call_1"}]"#)]
#[case::null_function(r#"[{"function":null}]"#)]
#[case::nothing_to_scan(r#"[{"function":{}}]"#)]
#[case::blank_arguments_and_no_name(r#"[{"function":{"arguments":" \n "}}]"#)]
#[case::empty_name_and_no_arguments(r#"[{"function":{"name":""}}]"#)]
#[tokio::test]
async fn a_call_with_nothing_to_scan_is_skipped(#[case] calls: &str) {
    let outcome = run(Setup::new(BLOCK_PLAIN), calls).await;
    outcome.result.as_ref().expect("nothing scanned");
    assert!(outcome.bodies.is_empty());
    assert!(outcome.audit.scan_ids.is_empty());
}

#[tokio::test]
async fn arguments_that_are_not_a_string_are_scanned_as_their_json_text() {
    let calls = r#"[{"function":{"name":"f","arguments":{"city":"Paris"}}}]"#;
    let error = run(Setup::new(BLOCK_PLAIN), calls)
        .await
        .result
        .expect_err("blocked");
    assert_eq!(error.status, 400);
    let outcome = run(Setup::new(ALLOW), calls).await;
    assert!(outcome.bodies[0].contains(&prompt_json("f\n{\"city\":\"Paris\"}")));
}

#[tokio::test]
async fn every_call_is_scanned_and_the_first_block_stops_the_rest() {
    let two =
        r#"[{"function":{"name":"a","arguments":"1"}},{"function":{"name":"b","arguments":"2"}}]"#;
    let allowed = run(Setup::new(ALLOW), two).await;
    allowed.result.expect("allowed");
    assert_eq!(allowed.bodies.len(), 2);
    let blocked = run(Setup::new(BLOCK_PLAIN), two).await;
    blocked.result.expect_err("blocked");
    assert_eq!(blocked.bodies.len(), 1);
}

#[tokio::test]
async fn a_transient_failure_with_fallback_allow_skips_the_call_unscanned() {
    let setup = Setup {
        status: StatusCode::SERVICE_UNAVAILABLE,
        fallback: FallbackAction::Allow,
        ..Setup::new("{}")
    };
    let outcome = run(setup, WEATHER).await;
    outcome.result.as_ref().expect("fallback allows");
    assert_eq!(outcome.audit.applied_guardrails, ["panw:unscanned"]);
    assert_eq!(outcome.audit.information.len(), 1);
    assert_eq!(arguments(&outcome, 0), Some(r#"{"city":"Paris"}"#));
}

#[tokio::test]
async fn a_transient_failure_with_fallback_block_is_a_500() {
    let setup = Setup {
        status: StatusCode::SERVICE_UNAVAILABLE,
        ..Setup::new("{}")
    };
    let error = run(setup, WEATHER).await.result.expect_err("blocked");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
}

#[tokio::test]
async fn a_missing_call_id_is_a_500_and_nothing_is_sent() {
    let setup = Setup {
        call_id: None,
        fallback: FallbackAction::Allow,
        ..Setup::new(ALLOW)
    };
    let outcome = run(setup, WEATHER).await;
    let error = outcome.result.as_ref().expect_err("blocked");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.category, "missing_call_id");
    assert!(outcome.bodies.is_empty());
}
