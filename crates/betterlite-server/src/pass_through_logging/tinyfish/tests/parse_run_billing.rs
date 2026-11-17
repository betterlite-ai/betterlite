#![cfg(test)]

use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;

use super::TINYFISH_MODEL_NAME;
use super::payload::build_logging_payload;
use super::records::parse_run;
use super::support::{
    decimal, detail_cost, detail_text, json, kwargs, logged_model, logged_provider, logging_object,
    run,
};

fn standard_response(outcome: &HandlerOutcome) -> &Json {
    match outcome.result.as_ref() {
        Some(PassThroughResponse::Standard(response)) => &response.response,
        _ => panic!("the payload logs a standard response"),
    }
}

#[test]
fn stamps_the_run_model_provider_and_cost() {
    let logging_obj = logging_object();
    let submitted = run(
        r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3, "result": {"ok": true}}"#,
    );
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &kwargs(&[]),
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
    assert_eq!(logged_provider(&logging_obj).as_deref(), Some("tinyfish"));
    assert_eq!(
        detail_text(&logging_obj, "model").as_deref(),
        Some(TINYFISH_MODEL_NAME)
    );
    assert_eq!(
        detail_text(&logging_obj, "custom_llm_provider").as_deref(),
        Some("tinyfish")
    );
    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("0.048"));
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
    assert_eq!(
        outcome.kwargs.entries.get("litellm_call_id"),
        Some(&Json::Text("call-1".to_owned()))
    );
    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&json("{}"))
    );
    assert_eq!(
        standard_response(&outcome),
        &json(
            r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3, "result": {"ok": true}}"#
        )
    );
}

#[test]
fn logs_the_raw_result_when_the_run_does_not_parse() {
    let logging_obj = logging_object();
    let outcome = build_logging_payload(
        &logging_obj,
        None,
        "raw upstream text",
        &kwargs(&[]),
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(
        standard_response(&outcome),
        &Json::Text("raw upstream text".to_owned())
    );
    assert_eq!(detail_cost(&logging_obj), None);
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
}

#[test]
fn bills_nothing_for_a_failed_run() {
    let logging_obj = logging_object();
    let submitted =
        parse_run(&json(r#"{"status": "FAILED", "num_of_steps": 3}"#)).expect("the run parses");
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &kwargs(&[]),
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(detail_cost(&logging_obj), None);
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
}

#[test]
fn keeps_the_callers_kwargs_beside_the_spend() {
    let logging_obj = logging_object();
    let caller = kwargs(&[
        ("litellm_call_id", Json::Text("call-9".to_owned())),
        ("team", Json::Text("team-1".to_owned())),
    ]);
    let submitted = run(r#"{"status": "COMPLETED", "num_of_steps": 2}"#);
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &caller,
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(
        outcome.kwargs.entries.get("litellm_call_id"),
        Some(&Json::Text("call-1".to_owned()))
    );
    assert_eq!(
        outcome.kwargs.entries.get("team"),
        Some(&Json::Text("team-1".to_owned()))
    );
    assert_eq!(
        outcome.kwargs.entries.get("model"),
        Some(&Json::Text(TINYFISH_MODEL_NAME.to_owned()))
    );
}

#[test]
fn keeps_a_callers_attribution_params() {
    let logging_obj = logging_object();
    let caller = kwargs(&[(
        "litellm_params",
        json(r#"{"metadata": {"user_api_key_alias": "team-key"}}"#),
    )]);
    let submitted = run(r#"{"status": "COMPLETED", "num_of_steps": 2}"#);
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &caller,
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&json(r#"{"metadata": {"user_api_key_alias": "team-key"}}"#))
    );
}

#[test]
fn falls_back_to_the_stored_params_for_an_empty_caller_value() {
    let logging_obj = logging_object();
    let caller = kwargs(&[("litellm_params", json("{}"))]);
    let submitted = run(r#"{"status": "COMPLETED", "num_of_steps": 2}"#);
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &caller,
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&json("{}"))
    );
}

#[test]
fn prices_at_the_resolved_rate() {
    let logging_obj = logging_object();
    let submitted = run(r#"{"status": "COMPLETED", "num_of_steps": 3}"#);
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &kwargs(&[]),
        decimal("0.5"),
    )
    .expect("the payload builds");

    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("1.5"));
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&json("1.5"))
    );
}

#[test]
fn logs_an_empty_run_as_an_empty_object() {
    let logging_obj = logging_object();
    let submitted = parse_run(&json("{}")).expect("the run parses");
    let outcome = build_logging_payload(
        &logging_obj,
        Some(&submitted),
        "",
        &kwargs(&[]),
        decimal("0.016"),
    )
    .expect("the payload builds");

    assert_eq!(standard_response(&outcome), &json("{}"));
    assert_eq!(detail_cost(&logging_obj), None);
}
