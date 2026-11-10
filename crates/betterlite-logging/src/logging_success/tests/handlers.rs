use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, chat_result, correlation, request, run_once, state,
};

#[rstest]
#[case::sync_twice("sync", false, "", false, 1)]
#[case::sync_stream_twice("sync", true, "", false, 1)]
#[case::async_twice("async", false, "", false, 1)]
#[case::async_stream_twice("async", true, "", false, 1)]
#[case::sync_on_async_call("sync", false, "acompletion", false, 0)]
#[case::sync_pass_through("sync", false, "pass_through", false, 0)]
#[case::sync_no_log("sync", false, "no-log", false, 0)]
#[case::async_no_log("async", false, "no-log", false, 0)]
#[case::sync_no_log_ignored("sync", false, "no-log", true, 1)]
#[case::async_no_log_ignored("async", false, "no-log", true, 1)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(
    #[case] path: &str,
    #[case] stream: bool,
    #[case] flag: &str,
    #[case] ignore_no_log: bool,
    #[case] expected: usize,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    fixture.add(&Arc::new(FakeLogger::new("langfuse", &calls)));
    let mut state = state(Some(stream));
    match flag {
        "pass_through" => state.call_type = "pass_through_endpoint".to_owned(),
        "" => {}
        key => {
            state
                .litellm_params
                .insert(key.to_owned(), Json::Bool(true));
        }
    }
    state.callback_policy.global_disable_no_log_param = ignore_no_log;
    let request = request(chat_result());
    let mut correlation = correlation();
    let handler = fixture.handler();
    run_once(path, &handler, &mut state, &mut correlation, &request).await;
    run_once(path, &handler, &mut state, &mut correlation, &request).await;
    assert_eq!(calls.events().len(), expected);
}

// an assembled stream result skips the has_logged gate on the async path only
#[rstest]
#[case::async_assembled_stream("async", "has_logged_async_success", 1)]
#[case::sync_stream_result("sync", "has_logged_sync_success", 0)]
#[tokio::test]
async fn already_logged_stream_gate(
    #[case] path: &str,
    #[case] key: &str,
    #[case] expected: usize,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    fixture.add(&Arc::new(FakeLogger::new("langfuse", &calls)));
    let mut state = state(Some(true));
    state
        .model_call_details
        .insert(key.to_owned(), Json::Bool(true));
    let mut correlation = correlation();
    let request = request(chat_result());
    run_once(
        path,
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &request,
    )
    .await;
    assert_eq!(calls.events().len(), expected);
}
