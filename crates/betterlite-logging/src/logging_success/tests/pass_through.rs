use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    CallLog, FakeLogger, Fixture, bare_request, chunk_result, correlation, request, run_once, state,
};
use crate::logging_success::assembled::ASYNC_STREAM_RESPONSE_KEY;
use crate::logging_success::payload_state::standard_logging_object;

#[rstest]
#[case::async_delivers_one_success_event("async", false, None, &["async_log_success_event"], 0)]
#[case::async_builds_the_missing_payload("async", true, None, &["async_log_success_event"], 1)]
#[case::async_keeps_a_known_cost("async", false, Some(5), &["async_log_success_event"], 0)]
#[case::sync_has_no_pass_through_branch("sync", false, None, &[], 0)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(
    #[case] path: &str,
    #[case] missing_payload: bool,
    #[case] existing_cost: Option<u8>,
    #[case] expected_events: &[&str],
    #[case] expected_builds: usize,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    fixture.add(&Arc::new(FakeLogger::new("langfuse", &calls)));
    let mut state = state(Some(true));
    state.call_type = "pass_through_endpoint".to_owned();
    if let Some(cost) = existing_cost {
        state
            .model_call_details
            .insert("response_cost".to_owned(), Json::Number(Number::from(cost)));
    }
    let request = if missing_payload {
        bare_request(chunk_result())
    } else {
        request(chunk_result())
    };
    let mut correlation = correlation();
    let handler = fixture.handler();
    run_once(path, &handler, &mut state, &mut correlation, &request).await;
    run_once(path, &handler, &mut state, &mut correlation, &request).await;
    assert_eq!(calls.events(), expected_events);
    assert_eq!(fixture.builder.builds(), expected_builds);
    assert_eq!(
        state
            .model_call_details
            .contains_key(ASYNC_STREAM_RESPONSE_KEY),
        path == "async"
    );
    if path == "async" {
        let expected_cost =
            existing_cost.map_or(Json::Null, |cost| Json::Number(Number::from(cost)));
        assert_eq!(
            state.model_call_details.get("response_cost"),
            Some(&expected_cost)
        );
        assert!(standard_logging_object(&state).unwrap().is_some());
    }
}
