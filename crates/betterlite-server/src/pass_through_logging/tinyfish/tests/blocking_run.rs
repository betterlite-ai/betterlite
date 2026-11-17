#![cfg(test)]

use betterlite_models::decisions::Json;

use super::TINYFISH_MODEL_NAME;
use super::billing::BlockingRun;
use super::support::{
    capture_logs, detail_cost, detail_text, fixture, json, kwargs, logged_model, logged_provider,
    logging_object, outcome_text,
};

const COMPLETED_BODY: &str =
    r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3, "result": {"ok": true}}"#;
const COMPLETED_TEXT: &str =
    r#"{"run_id":"run-1","status":"COMPLETED","num_of_steps":3,"result":{"ok":true}}"#;
const FAILED_BODY: &str = r#"{"run_id": "run-1", "status": "FAILED", "num_of_steps": 3}"#;
const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;

#[test]
fn bills_the_terminal_run_from_the_response_body() {
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let body = json(COMPLETED_BODY);
    let caller = kwargs(&[("team", Json::Text("team-1".to_owned()))]);
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: COMPLETED_BODY,
        kwargs: &caller,
    });

    assert_eq!(outcome_text(&outcome), COMPLETED_TEXT);
    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("0.048"));
    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
    assert_eq!(logged_provider(&logging_obj).as_deref(), Some("tinyfish"));
    assert_eq!(
        detail_text(&logging_obj, "custom_llm_provider").as_deref(),
        Some("tinyfish")
    );
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
    assert_eq!(
        outcome.kwargs.entries.get("model"),
        Some(&Json::Text(TINYFISH_MODEL_NAME.to_owned()))
    );
    assert_eq!(
        outcome.kwargs.entries.get("litellm_call_id"),
        Some(&Json::Text("call-1".to_owned()))
    );
    assert_eq!(
        outcome.kwargs.entries.get("team"),
        Some(&Json::Text("team-1".to_owned()))
    );
}

// The blocking answer is already the terminal run, so nothing polls and nothing dispatches again.
#[test]
fn makes_no_poll_request_and_no_success_dispatch() {
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let body = json(COMPLETED_BODY);
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: COMPLETED_BODY,
        kwargs: &kwargs(&[]),
    });

    assert_eq!(outcome_text(&outcome), COMPLETED_TEXT);
    assert!(fixture.transport.requests().is_empty());
    assert!(fixture.dispatch.calls().is_empty());
}

#[test]
fn a_failed_run_bills_nothing() {
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let body = json(FAILED_BODY);
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: FAILED_BODY,
        kwargs: &kwargs(&[]),
    });

    assert_eq!(
        outcome_text(&outcome),
        r#"{"run_id":"run-1","status":"FAILED","num_of_steps":3}"#
    );
    assert_eq!(detail_cost(&logging_obj), None);
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
}

#[test]
fn a_pending_run_bills_nothing() {
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let body = json(PENDING_BODY);
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: PENDING_BODY,
        kwargs: &kwargs(&[]),
    });

    assert_eq!(detail_cost(&logging_obj), None);
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
}

#[test]
fn logs_the_raw_result_when_the_body_is_absent() {
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: None,
        result: "raw upstream text",
        kwargs: &kwargs(&[]),
    });

    assert_eq!(outcome_text(&outcome), "raw upstream text");
    assert_eq!(detail_cost(&logging_obj), None);
    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
}

#[test]
fn a_body_of_the_wrong_shape_logs_the_raw_result() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let body = json("[1, 2]");
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: "raw upstream text",
        kwargs: &kwargs(&[]),
    });

    assert_eq!(outcome_text(&outcome), "raw upstream text");
    assert_eq!(detail_cost(&logging_obj), None);
    assert!(logs.contents().contains("unexpected run object shape"));
}
