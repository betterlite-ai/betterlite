use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::support::MarkingHook;
use super::support::Stub;
use super::support::chat_request;
use super::support::chat_stream;
use super::support::fixture;
use super::support::request;
use super::support::response;
use super::support::run_sync;
use super::support::value;
use crate::call_setup::hooks::DeploymentHook;
use crate::call_wrapper::state::WrapperOptions;

#[test]
fn sync_stream_returns_the_chunks_and_updates_metadata() {
    let mut fixture = fixture();
    let stream = chat_stream(value(r#"{"id":"stream-1"}"#), false);
    let stub = Stub::responding("completion", stream.clone());
    let outcome = run_sync(&mut fixture, &stub, chat_request(r#"{"stream":true}"#))
        .expect("the stream returns");
    assert_eq!(outcome, stream);
    assert_eq!(fixture.metadata.count(), 1);
    assert!(fixture.logging.events().is_empty());
    assert!(fixture.cache.stored().is_empty());
}

#[test]
fn sync_complete_response_assembles_the_stream() {
    let mut fixture = fixture();
    let stub = Stub::responding(
        "completion",
        chat_stream(value(r#"{"assembled":true}"#), false),
    );
    let request = chat_request(r#"{"stream":true,"complete_response":true}"#);
    let outcome = run_sync(&mut fixture, &stub, request).expect("the stream is assembled");
    assert_eq!(outcome, response(value(r#"{"assembled":true}"#)));
    assert_eq!(fixture.metadata.count(), 0);
    assert!(fixture.logging.events().is_empty());
}

#[tokio::test]
async fn async_stream_is_wrapped_for_the_cache() {
    let mut fixture = fixture().with_cache();
    let stream = chat_stream(value(r#"{"id":"stream-1"}"#), false);
    let stub = Stub::responding("acompletion", stream.clone());
    let outcome = fixture
        .wrapper
        .call(
            &stub,
            chat_request(r#"{"stream":true}"#),
            WrapperOptions::default(),
        )
        .await
        .expect("the stream returns");
    assert_eq!(outcome, stream);
    assert!(fixture.cache.was_wrapped());
    assert_eq!(fixture.metadata.count(), 1);
    assert!(fixture.cache.stored().is_empty());
}

#[tokio::test]
async fn async_complete_response_assembles_without_wrapping() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::responding(
        "acompletion",
        chat_stream(value(r#"{"assembled":true}"#), false),
    );
    let request = chat_request(r#"{"stream":true,"complete_response":true}"#);
    let outcome = fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect("the stream is assembled");
    assert_eq!(outcome, response(value(r#"{"assembled":true}"#)));
    assert!(!fixture.cache.was_wrapped());
    assert_eq!(fixture.metadata.count(), 0);
}

#[tokio::test]
async fn async_converted_stream_runs_the_success_hook() {
    let hook: Arc<dyn DeploymentHook> = Arc::new(MarkingHook::new("marker", "hooked"));
    let mut fixture = fixture().with_hooks(vec![hook]).with_cache();
    let stub = Stub::responding("acompletion", chat_stream(value(r#"{"x":1}"#), true));
    let outcome = fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the stream returns");
    let expected = chat_stream(value(r#"{"x":1,"hooked":true}"#), true);
    assert_eq!(outcome, expected);
    assert!(fixture.cache.was_wrapped());
}

#[tokio::test]
async fn async_converted_stream_without_a_mock_stream_skips_the_hook() {
    let hook: Arc<dyn DeploymentHook> = Arc::new(MarkingHook::new("marker", "hooked"));
    let mut fixture = fixture().with_hooks(vec![hook]).with_cache();
    let stub = Stub::responding("acompletion", chat_stream(value(r#"{"x":1}"#), false));
    let outcome = fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the stream returns");
    assert_eq!(outcome, chat_stream(value(r#"{"x":1}"#), false));
}

#[tokio::test]
async fn async_arealtime_returns_without_the_post_call_steps() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::responding("arealtime", response(value(r#"{"id":"rt-1"}"#)));
    let outcome = fixture
        .wrapper
        .call(
            &stub,
            request(r#"{"model":"gpt-4o"}"#),
            WrapperOptions::default(),
        )
        .await
        .expect("the realtime call returns");
    assert_eq!(outcome, response(value(r#"{"id":"rt-1"}"#)));
    assert_eq!(fixture.metadata.count(), 0);
    assert_eq!(fixture.scheduler.count(), 0);
    assert!(fixture.cache.stored().is_empty());
    assert_eq!(stub.call_count(), 1);
}

#[test]
fn a_streamed_result_is_not_stored_by_the_sync_path() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::responding(
        "completion",
        chat_stream(value(r#"{"id":"stream-1"}"#), false),
    );
    run_sync(&mut fixture, &stub, chat_request(r#"{"stream":true}"#)).expect("the stream returns");
    assert!(fixture.cache.stored().is_empty());
    assert_eq!(fixture.cache.lookups().len(), 1);
}
