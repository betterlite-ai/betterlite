use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{CollectorFailure, FakeCollector, HandlerFixture, json, state_with};
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_streams::flush::{async_flush_passthrough, flush_passthrough};
use crate::logging_success::assembled::{ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE};
use crate::logging_success::types::SuccessResult;

type Behavior = fn() -> Pin<Box<dyn Future<Output = ()> + Send>>;

fn passthrough_state() -> crate::logging_state::LoggingState {
    state_with(Some(true), "pass_through_endpoint", "gpt-4o")
}

fn response() -> Json {
    json(r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[]}"#)
}

fn sync_logs_the_collected_response() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::returning(SuccessResult::Chat(response()));
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector).unwrap();
        assert_eq!(
            state.model_call_details.get(COMPLETE_STREAMING_RESPONSE),
            Some(&response())
        );
        assert!(
            !state
                .model_call_details
                .contains_key(ASYNC_STREAM_RESPONSE_KEY)
        );
        assert_eq!(fixture.builder.responses(), vec![response()]);
        assert_eq!(
            state.model_call_details.get("log_event_type"),
            Some(&Json::Text("successful_api_call".to_owned()))
        );
        assert_eq!(
            state.model_call_details.get("end_time"),
            Some(&Json::Text("2024-01-01T00:00:02+00:00".to_owned()))
        );
        assert_eq!(state.model_call_details.get("cache_hit"), Some(&Json::Null));
        assert_eq!(collector.seen_model(), Some("gpt-4o".to_owned()));
    })
}

fn sync_skips_logging_without_a_collected_response() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::empty();
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector).unwrap();
        assert!(!state.model_call_details.contains_key("log_event_type"));
        assert_eq!(fixture.builder.responses(), Vec::new());
        assert_eq!(collector.seen_model(), Some("gpt-4o".to_owned()));
    })
}

fn sync_returns_the_collector_error() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::failing("boom");
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        let outcome =
            flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector);
        assert!(matches!(outcome, Err(CollectorFailure { message: "boom" })));
        assert!(!state.model_call_details.contains_key("log_event_type"));
    })
}

fn sync_keeps_a_pass_through_chunk_out_of_the_stream_keys()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::returning(SuccessResult::Chunk(response()));
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector).unwrap();
        assert!(
            !state
                .model_call_details
                .contains_key(COMPLETE_STREAMING_RESPONSE)
        );
        assert!(
            !state
                .model_call_details
                .contains_key(ASYNC_STREAM_RESPONSE_KEY)
        );
        assert_eq!(fixture.builder.responses(), Vec::new());
    })
}

fn async_logs_the_assembled_response() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let complete = json(
            r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#,
        );
        let fixture = HandlerFixture::new(Some(complete.clone()));
        let collector = FakeCollector::returning(SuccessResult::Chunk(response()));
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        async_flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector)
            .await
            .unwrap();
        assert_eq!(
            state.model_call_details.get(ASYNC_STREAM_RESPONSE_KEY),
            Some(&complete)
        );
        assert!(
            !state
                .model_call_details
                .contains_key(COMPLETE_STREAMING_RESPONSE)
        );
        assert_eq!(fixture.builder.responses(), vec![complete]);
    })
}

fn async_keeps_a_pass_through_result_as_the_logged_response()
-> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::returning(SuccessResult::Chunk(response()));
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        async_flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector)
            .await
            .unwrap();
        assert_eq!(
            state.model_call_details.get(ASYNC_STREAM_RESPONSE_KEY),
            Some(&response())
        );
        assert_eq!(fixture.builder.responses(), vec![response()]);
        assert_eq!(
            state.model_call_details.get("response_cost"),
            Some(&Json::Null)
        );
    })
}

fn async_skips_logging_without_a_collected_response() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::empty();
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        async_flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector)
            .await
            .unwrap();
        assert!(!state.model_call_details.contains_key("log_event_type"));
        assert!(
            !state
                .model_call_details
                .contains_key(ASYNC_STREAM_RESPONSE_KEY)
        );
    })
}

fn async_returns_the_collector_error() -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async {
        let fixture = HandlerFixture::new(None);
        let collector = FakeCollector::failing("boom");
        let mut state = passthrough_state();
        let mut correlation = CorrelationContext::default();
        let outcome =
            async_flush_passthrough(&fixture.handler(), &mut state, &mut correlation, &collector)
                .await;
        assert!(matches!(outcome, Err(CollectorFailure { message: "boom" })));
        assert!(!state.model_call_details.contains_key("log_event_type"));
    })
}

#[rstest]
#[case::flush_sync_logs_the_collected_response(sync_logs_the_collected_response)]
#[case::flush_sync_skips_without_a_response(sync_skips_logging_without_a_collected_response)]
#[case::flush_sync_returns_the_collector_error(sync_returns_the_collector_error)]
#[case::flush_sync_keeps_a_chunk_out_of_the_stream_keys(
    sync_keeps_a_pass_through_chunk_out_of_the_stream_keys
)]
#[case::flush_async_logs_the_assembled_response(async_logs_the_assembled_response)]
#[case::flush_async_keeps_a_pass_through_result(
    async_keeps_a_pass_through_result_as_the_logged_response
)]
#[case::flush_async_skips_without_a_response(async_skips_logging_without_a_collected_response)]
#[case::flush_async_returns_the_collector_error(async_returns_the_collector_error)]
#[tokio::test]
async fn logging_streams_behavior_tests_behavior(#[case] behavior: Behavior) {
    behavior().await;
}
