use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallLog, FakeLogger, Fixture, bare_request, chat_result, chunk_result, correlation, payload,
    request, state,
};
use crate::logging_success::dispatch::dispatch_success_handlers;

fn source_scenario(scenario: &str) -> &str {
    scenario.strip_prefix("bare_").unwrap_or(scenario)
}

#[rstest]
#[case::sync_success("sync_success", "log_success_event", 0)]
#[case::sync_stream_chunk("sync_stream_chunk", "log_stream_event", 0)]
#[case::sync_assembled_stream("sync_assembled_stream", "log_success_event", 1)]
#[case::async_success("async_success", "async_log_success_event", 0)]
#[case::async_stream_chunk("async_stream_chunk", "async_log_stream_event", 0)]
#[case::async_assembled_stream("async_assembled_stream", "async_log_success_event", 1)]
#[case::bare_sync_success("bare_sync_success", "log_success_event", 1)]
#[case::bare_async_success("bare_async_success", "async_log_success_event", 1)]
#[case::bare_sync_stream_chunk("bare_sync_stream_chunk", "log_stream_event", 1)]
#[case::bare_async_stream_chunk("bare_async_stream_chunk", "async_log_stream_event", 1)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(
    #[case] scenario: &str,
    #[case] expected_event: &str,
    #[case] expected_builds: usize,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    let logger = Arc::new(FakeLogger::new("langfuse", &calls));
    fixture.add(&logger);
    let source = source_scenario(scenario);
    let stream = source.contains("stream") || source.contains("chunk");
    let mut state = state(Some(stream));
    if source.starts_with("async") {
        state
            .litellm_params
            .insert("acompletion".to_owned(), Json::Bool(true));
    }
    let result = if source.contains("chunk") {
        chunk_result()
    } else {
        chat_result()
    };
    let request = if scenario.starts_with("bare_") {
        bare_request(result)
    } else {
        request(result)
    };
    let mut correlation = correlation();
    dispatch_success_handlers(&fixture.handler(), &mut state, &mut correlation, &request).await;
    assert_eq!(calls.events(), [expected_event]);
    assert_eq!(calls.calls()[0].stream, stream);
    assert_eq!(fixture.builder.builds(), expected_builds);
    let expected_messages = payload().messages;
    assert_eq!(calls.calls()[0].messages, expected_messages);
}
