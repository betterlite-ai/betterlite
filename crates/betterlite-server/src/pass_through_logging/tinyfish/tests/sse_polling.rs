#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::TINYFISH_MODEL_NAME;
use super::billing::SseRun;
use super::sse::sse_poller_spawned;
use super::support::{
    ScriptedReply, detail_cost, dispatch_handle, fixture, fixture_with_credentials, json,
    shared_logging_object, standard_text, start_time,
};

const PENDING_BODY: &str = r#"{"run_id": "run-7", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-7", "status": "COMPLETED", "num_of_steps": 2}"#;
const COMPLETED_TEXT: &str = r#"{"run_id":"run-7","status":"COMPLETED","num_of_steps":2}"#;

#[tokio::test]
async fn the_sse_poller_bills_the_terminal_run() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id: "run-7".to_owned(),
        logging_obj: Arc::clone(&logging_obj),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    assert!(sse_poller_spawned(&logging_obj));
    handle.await.expect("the poller finishes");

    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests.first().expect("one request").url,
        "https://agent.tinyfish.ai/v1/runs/run-7?screenshots=none"
    );
    let calls = fixture.dispatch.calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), COMPLETED_TEXT);
    assert_eq!(
        call.kwargs.entries.get("response_cost"),
        Some(&json("0.032"))
    );
    assert_eq!(call.model, TINYFISH_MODEL_NAME);
    assert_eq!(call.provider.as_deref(), Some("tinyfish"));
    assert_eq!(call.call_id, "call-1");
    assert!(!call.cache_hit);
    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("0.032"));
}

#[tokio::test]
async fn the_sse_poller_runs_through_a_pending_run() {
    let fixture = fixture(vec![
        ScriptedReply::Run(200, PENDING_BODY),
        ScriptedReply::Run(200, COMPLETED_BODY),
    ]);
    let logging_obj = shared_logging_object();
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id: "run-7".to_owned(),
        logging_obj,
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert_eq!(fixture.transport.requests().len(), 2);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), COMPLETED_TEXT);
}

// The router reads the marker after the stream ends, so it must be set before the poller finishes.
#[tokio::test]
async fn the_marker_is_set_while_the_poller_still_runs() {
    let fixture = fixture(vec![ScriptedReply::Hanging]);
    let logging_obj = shared_logging_object();
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id: "run-7".to_owned(),
        logging_obj: Arc::clone(&logging_obj),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });

    assert!(sse_poller_spawned(&logging_obj));
    assert!(!handle.is_finished());
    handle.abort();
    assert!(
        handle
            .await
            .expect_err("the poller was cancelled")
            .is_cancelled()
    );
}

// LiteLLM hands the SSE poller an empty result and empty kwargs, so the payload has only the spend keys.
#[tokio::test]
async fn an_sse_run_without_credentials_logs_an_empty_result() {
    let fixture = fixture_with_credentials(Vec::new(), None);
    let logging_obj = shared_logging_object();
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id: "run-7".to_owned(),
        logging_obj: Arc::clone(&logging_obj),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert!(fixture.transport.requests().is_empty());
    assert!(sse_poller_spawned(&logging_obj));
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), "");
    assert_eq!(call.kwargs.entries.get("response_cost"), Some(&Json::Null));
    assert_eq!(
        call.kwargs.entries.get("model"),
        Some(&Json::Text(TINYFISH_MODEL_NAME.to_owned()))
    );
    assert_eq!(
        call.kwargs.entries.get("litellm_call_id"),
        Some(&Json::Text("call-1".to_owned()))
    );
    assert_eq!(call.kwargs.entries.len(), 5);
}

// The stored params come from the request, not from kwargs the SSE path never had.
#[tokio::test]
async fn the_sse_poller_keeps_the_stored_request_params() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    logging_obj.edit(&mut |state| {
        state.model_call_details.insert(
            "litellm_params".to_owned(),
            json(r#"{"metadata": {"user_api_key_alias": "stored-key"}}"#),
        );
    });
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id: "run-7".to_owned(),
        logging_obj,
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        call.kwargs.entries.get("litellm_params"),
        Some(&json(
            r#"{"metadata": {"user_api_key_alias": "stored-key"}}"#
        ))
    );
}
