#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::TINYFISH_MODEL_NAME;
use super::billing::{AsyncRun, BlockingRun, SseRun};
use super::support::{
    ScriptedReply, detail_cost, detail_text, dispatch_handle, fixture, json, kwargs, logged_model,
    logged_provider, logging_object, shared_logging_object, start_time,
};

const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;

fn assert_spend_fields(entries: &IndexMap<String, Json>) {
    assert_eq!(
        entries.get("model"),
        Some(&Json::Text(TINYFISH_MODEL_NAME.to_owned()))
    );
    assert_eq!(
        entries.get("custom_llm_provider"),
        Some(&Json::Text("tinyfish".to_owned()))
    );
    assert_eq!(entries.get("response_cost"), Some(&json("0.048")));
    assert_eq!(
        entries.get("litellm_call_id"),
        Some(&Json::Text("call-1".to_owned()))
    );
}

#[test]
fn the_blocking_payload_keeps_all_four_fields() {
    let fixture = fixture(Vec::new());
    let logging_obj = logging_object();
    let body = json(COMPLETED_BODY);
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: COMPLETED_BODY,
        kwargs: &kwargs(&[]),
    });

    assert_spend_fields(&outcome.kwargs.entries);
    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
    assert_eq!(logged_provider(&logging_obj).as_deref(), Some("tinyfish"));
    assert_eq!(
        detail_text(&logging_obj, "model").as_deref(),
        Some(TINYFISH_MODEL_NAME)
    );
    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("0.048"));
}

#[tokio::test]
async fn the_poller_payload_keeps_all_four_fields() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: Arc::clone(&logging_obj),
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[("team", Json::Text("team-1".to_owned()))]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_spend_fields(&call.kwargs.entries);
    assert_eq!(
        call.kwargs.entries.get("team"),
        Some(&Json::Text("team-1".to_owned()))
    );
    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("0.048"));
}

#[tokio::test]
async fn the_sse_payload_keeps_all_four_fields() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    fixture
        .billing
        .start_sse_run_billing(SseRun {
            run_id: "run-1".to_owned(),
            logging_obj: Arc::clone(&logging_obj),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_spend_fields(&call.kwargs.entries);
    assert_eq!(logged_model(&logging_obj), TINYFISH_MODEL_NAME);
    assert_eq!(logged_provider(&logging_obj).as_deref(), Some("tinyfish"));
    assert_eq!(
        detail_text(&logging_obj, "custom_llm_provider").as_deref(),
        Some("tinyfish")
    );
}

// LiteLLM's _handle_logging drops a poller's cache_hit and logs the row as a fresh call.
#[tokio::test]
async fn a_cached_request_is_dispatched_as_not_cached() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    logging_obj.edit(&mut |state| {
        state
            .model_call_details
            .insert("cache_hit".to_owned(), Json::Bool(true));
    });
    fixture
        .billing
        .start_sse_run_billing(SseRun {
            run_id: "run-1".to_owned(),
            logging_obj,
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert!(!call.cache_hit);
}

// The poller has no request kwargs, so the params stored with the request are what attribution reads.
#[tokio::test]
async fn the_stored_params_reach_the_poller_payload() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    logging_obj.edit(&mut |state| {
        state.model_call_details.insert(
            "litellm_params".to_owned(),
            json(r#"{"metadata": {"team_id": "team-1"}}"#),
        );
    });
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj,
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        call.kwargs.entries.get("litellm_params"),
        Some(&json(r#"{"metadata": {"team_id": "team-1"}}"#))
    );
}
