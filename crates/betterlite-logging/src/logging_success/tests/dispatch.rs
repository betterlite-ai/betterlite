use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, chat_result, chunk_result, correlation, request, state,
};
use crate::logging_success::assembled::FINAL_STREAM_DISPATCH_MARKER;
use crate::logging_success::dispatch::{
    dispatch_success_handlers, handle_sync_success_callbacks_for_async_calls,
};

#[rstest]
#[case::sync_sdk("sync_sdk")]
#[case::async_sdk("async_sdk")]
#[case::passthrough("passthrough")]
#[case::prefer_async("prefer_async")]
#[case::assembled_twice("assembled_twice")]
#[case::chunk_twice("chunk_twice")]
#[case::non_stream_twice("non_stream_twice")]
#[case::sync_on_async_unknown("sync_on_async_unknown")]
#[case::sync_on_async_known("sync_on_async_known")]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(#[case] scenario: &str) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    let logger = Arc::new(FakeLogger::new("langfuse", &calls));
    fixture.add(&logger);
    if scenario == "sync_on_async_known" {
        fixture.set_known(vec!["legacy_hook".to_owned()]);
    }
    let stream = matches!(scenario, "assembled_twice" | "chunk_twice");
    let mut state = state(Some(stream));
    let mut request = request(if scenario == "chunk_twice" {
        chunk_result()
    } else {
        chat_result()
    });
    if scenario == "passthrough" {
        state.call_type = "pass_through_endpoint".to_owned();
    }
    if matches!(
        scenario,
        "async_sdk"
            | "assembled_twice"
            | "chunk_twice"
            | "sync_on_async_unknown"
            | "sync_on_async_known"
    ) {
        state
            .litellm_params
            .insert("acompletion".to_owned(), Json::Bool(true));
    }
    if scenario == "prefer_async" {
        request.prefer_async_handlers = true;
    }
    if scenario.starts_with("sync_on_async") {
        state.dynamic_callbacks.success = Some(vec!["legacy_hook".to_owned()]);
    }
    let mut correlation = correlation();
    let handler = fixture.handler();
    let dispatched_sync = if scenario.starts_with("sync_on_async") {
        handle_sync_success_callbacks_for_async_calls(
            &handler,
            &mut state,
            &mut correlation,
            &request,
        )
    } else {
        false
    };
    if !scenario.starts_with("sync_on_async") {
        dispatch_success_handlers(&handler, &mut state, &mut correlation, &request).await;
        if matches!(
            scenario,
            "assembled_twice" | "chunk_twice" | "non_stream_twice"
        ) {
            dispatch_success_handlers(&handler, &mut state, &mut correlation, &request).await;
        }
    }
    match scenario {
        "sync_sdk" | "non_stream_twice" => assert_eq!(calls.events(), ["log_success_event"]),
        "async_sdk" | "passthrough" | "prefer_async" | "assembled_twice" => {
            assert_eq!(calls.events(), ["async_log_success_event"]);
        }
        "chunk_twice" => {
            assert_eq!(
                calls.events(),
                ["async_log_stream_event", "async_log_stream_event"]
            );
        }
        "sync_on_async_unknown" => {
            assert!(dispatched_sync);
            assert!(calls.events().is_empty());
            assert_eq!(
                state.model_call_details.get("has_logged_sync_success"),
                Some(&Json::Bool(true))
            );
        }
        "sync_on_async_known" => {
            assert!(!dispatched_sync);
            assert!(calls.events().is_empty());
            assert!(
                !state
                    .model_call_details
                    .contains_key("has_logged_sync_success")
            );
        }
        other => panic!("unexpected scenario {other}"),
    }
    if scenario == "assembled_twice" {
        assert_eq!(
            state.model_call_details.get(FINAL_STREAM_DISPATCH_MARKER),
            Some(&Json::Bool(true))
        );
    }
}
