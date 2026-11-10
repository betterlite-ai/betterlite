use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, chat_result, correlation, request, run_once, state,
};

#[rstest]
#[case::sync_proxy_logger_runs("sync", "LiteLLM_PROXY_Logger", 1)]
#[case::async_proxy_logger_runs("async", "LiteLLM_PROXY_Logger", 1)]
#[case::sync_external_logger_skipped("sync", "langfuse", 0)]
#[case::async_external_logger_skipped("async", "langfuse", 0)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(
    #[case] path: &str,
    #[case] name: &str,
    #[case] expected: usize,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    fixture.add(&Arc::new(FakeLogger::new(name, &calls)));
    let mut state = state(Some(false));
    state
        .litellm_params
        .insert("no-log".to_owned(), Json::Bool(true));
    let call = request(chat_result());
    let mut correlation = correlation();
    run_once(
        path,
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &call,
    )
    .await;
    assert_eq!(calls.events().len(), expected);
}
