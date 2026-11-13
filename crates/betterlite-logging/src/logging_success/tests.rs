#![cfg(test)]

mod context;
mod delivery;
mod dispatch;
mod errors;
mod handlers;
mod helpers;
mod message_logging;
mod pass_through;
mod privacy;
mod proxy_logger;
mod redaction;
mod response_shapes;
mod support;

use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use self::support::{
    CallLog, FakeLogger, Fixture, chat_result, correlation, parse, request, run_once, state,
};

#[rstest]
#[case::sync_object("sync", Some(r#"{"run_name": "my-run"}"#), r#"{"run_name": "my-run"}"#)]
#[case::async_object(
    "async",
    Some(r#"{"run_name": "my-run"}"#),
    r#"{"run_name": "my-run"}"#
)]
#[case::text_is_empty("async", Some(r#""my-run""#), "{}")]
#[case::absent_is_empty("sync", None, "{}")]
#[tokio::test]
async fn the_event_carries_litellm_params_metadata(
    #[case] path: &str,
    #[case] metadata: Option<&str>,
    #[case] expected: &str,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    fixture.add(&Arc::new(FakeLogger::new("langsmith", &calls)));
    let mut state = state(Some(false));
    if let Some(text) = metadata {
        state
            .litellm_params
            .insert("metadata".to_owned(), parse(text));
    }
    let mut correlation = correlation();
    let handler = fixture.handler();
    run_once(
        path,
        &handler,
        &mut state,
        &mut correlation,
        &request(chat_result()),
    )
    .await;
    let carried: Vec<Json> = calls
        .calls()
        .into_iter()
        .map(|call| Json::Object(call.litellm_metadata))
        .collect();
    assert_eq!(carried, [parse(expected)]);
}
