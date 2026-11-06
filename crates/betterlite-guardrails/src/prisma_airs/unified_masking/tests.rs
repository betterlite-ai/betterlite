#![cfg(test)]

use axum::http::StatusCode;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use rstest::rstest;

use crate::panw_prisma_airs::audit::Direction;
use crate::prisma_airs::support::{Run, execute, inputs, prompt_json, request_data, response_json};

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const ALLOW_MASKED: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1","prompt_masked_data":{"data":"MASKED"}}"#;
const ALLOW_EMPTY_MASK: &str = r#"{"action":"allow","prompt_masked_data":{"data":""}}"#;
const BLOCK_MASKED: &str = r#"{"action":"block","category":"pii","scan_id":"scan-2","prompt_masked_data":{"data":"MASKED"}}"#;
const BLOCK_MASKED_RESPONSE: &str = r#"{"action":"block","category":"pii","scan_id":"scan-2","response_masked_data":{"data":"MASKED"}}"#;
const BLOCK_PLAIN: &str = r#"{"action":"block","category":"injection","scan_id":"scan-3"}"#;

#[tokio::test]
async fn an_allowed_scan_takes_the_masked_text_whatever_the_flags_say() {
    let outcome = execute(
        Run::new(ALLOW_MASKED),
        inputs(&["secret"]),
        request_data(),
        None,
    )
    .await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.inputs.texts, ["MASKED"]);
    assert_eq!(outcome.audit.scan_ids, ["scan-1"]);
    assert_eq!(outcome.audit.applied_guardrails, ["panw"]);
    assert_eq!(outcome.bodies.len(), 1);
}

#[rstest]
#[case::no_masked_data(ALLOW)]
#[case::empty_masked_data(ALLOW_EMPTY_MASK)]
#[tokio::test]
async fn an_allowed_scan_without_usable_masked_data_keeps_the_text(#[case] reply: &str) {
    let outcome = execute(Run::new(reply), inputs(&["secret"]), request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.inputs.texts, ["secret"]);
}

#[tokio::test]
async fn a_blocked_request_with_masking_on_takes_the_masked_text() {
    let run = Run {
        mask_request: true,
        ..Run::new(BLOCK_MASKED)
    };
    let outcome = execute(run, inputs(&["secret"]), request_data(), None).await;
    outcome.result.as_ref().expect("masked instead of blocked");
    assert_eq!(outcome.inputs.texts, ["MASKED"]);
}

#[tokio::test]
async fn a_blocked_request_without_masking_is_a_400() {
    let outcome = execute(
        Run::new(BLOCK_PLAIN),
        inputs(&["secret"]),
        request_data(),
        None,
    )
    .await;
    let error = outcome.result.as_ref().expect_err("blocked");
    assert_eq!(error.status(), 400);
    let detail = serde_json::to_string(&error.detail().expect("detail")).expect("detail json");
    assert!(detail.contains(r#""code":"panw_prisma_airs_blocked""#));
    assert!(detail.contains(r#""category":"injection""#));
    assert_eq!(outcome.inputs.texts, ["secret"]);
}

#[tokio::test]
async fn a_blocked_response_uses_the_response_masked_data_and_code() {
    let run = Run {
        direction: Direction::Response,
        mask_response: true,
        ..Run::new(BLOCK_MASKED_RESPONSE)
    };
    let outcome = execute(run, inputs(&["secret"]), request_data(), None).await;
    outcome.result.as_ref().expect("masked instead of blocked");
    assert_eq!(outcome.inputs.texts, ["MASKED"]);
    assert_eq!(outcome.bodies.len(), 1);
    assert!(outcome.bodies[0].contains(&response_json("secret")));

    let blocked = Run {
        direction: Direction::Response,
        ..Run::new(BLOCK_PLAIN)
    };
    let blocked_outcome = execute(blocked, inputs(&["secret"]), request_data(), None).await;
    let error = blocked_outcome.result.as_ref().expect_err("blocked");
    let detail = serde_json::to_string(&error.detail().expect("detail")).expect("detail json");
    assert!(detail.contains(r#""code":"panw_prisma_airs_response_blocked""#));
}

#[rstest]
#[case::request_flag_only_on_a_response(Direction::Response, true, false)]
#[case::response_flag_only_on_a_request(Direction::Request, false, true)]
#[tokio::test]
async fn the_other_side_mask_flag_does_not_apply(
    #[case] direction: Direction,
    #[case] mask_request: bool,
    #[case] mask_response: bool,
) {
    let run = Run {
        direction,
        mask_request,
        mask_response,
        ..Run::new(BLOCK_MASKED)
    };
    let outcome = execute(run, inputs(&["secret"]), request_data(), None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocked").status(), 400);
    assert_eq!(outcome.inputs.texts, ["secret"]);
}

#[tokio::test]
async fn a_transient_failure_with_fallback_allow_keeps_the_text_unscanned() {
    let run = Run {
        status: StatusCode::SERVICE_UNAVAILABLE,
        fallback: FallbackAction::Allow,
        ..Run::new("{}")
    };
    let outcome = execute(run, inputs(&["secret"]), request_data(), None).await;
    outcome.result.as_ref().expect("fallback allows");
    assert_eq!(outcome.inputs.texts, ["secret"]);
    assert_eq!(outcome.audit.applied_guardrails, ["panw:unscanned", "panw"]);
    assert_eq!(outcome.audit.information.len(), 1);
}

#[tokio::test]
async fn a_transient_failure_without_fallback_is_a_500() {
    let run = Run {
        status: StatusCode::SERVICE_UNAVAILABLE,
        ..Run::new("{}")
    };
    let outcome = execute(run, inputs(&["secret"]), request_data(), None).await;
    assert_eq!(outcome.result.as_ref().expect_err("blocks").status(), 500);
}

#[tokio::test]
async fn blank_texts_are_returned_without_a_scan() {
    let outcome = execute(
        Run::new(BLOCK_PLAIN),
        inputs(&["", "   "]),
        request_data(),
        None,
    )
    .await;
    outcome.result.as_ref().expect("nothing to scan");
    assert_eq!(outcome.inputs.texts, ["", "   "]);
    assert!(outcome.bodies.is_empty());
    assert!(outcome.audit.scan_ids.is_empty());
}

#[tokio::test]
async fn the_scan_text_is_sent_as_a_prompt_with_the_prepared_metadata() {
    let mut data = request_data();
    data.user = Some("alice".to_owned());
    data.model = Some("gpt-4o".to_owned());
    let outcome = execute(Run::new(ALLOW), inputs(&["hello"]), data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies[0].contains(&prompt_json("hello")));
    assert!(outcome.bodies[0].contains(r#""app_user":"alice""#));
    assert!(outcome.bodies[0].contains(r#""ai_model":"gpt-4o""#));
}
