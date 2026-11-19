use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::support::MarkingHook;
use super::support::Stub;
use super::support::chat_request;
use super::support::chat_stream;
use super::support::fixture;
use super::support::response;
use super::support::run_sync;
use super::support::value;
use crate::call_setup::hooks::DeploymentHook;
use crate::call_wrapper::state::WrapperOptions;

#[test]
fn sync_call_returns_the_stub_result_with_one_call_and_a_call_id() {
    let mut fixture = fixture();
    let body = value(r#"{"id":"chat-1"}"#);
    let stub = Stub::responding("completion", response(body.clone()));
    let outcome = run_sync(&mut fixture, &stub, chat_request("{}")).expect("the call succeeds");
    assert_eq!(outcome, response(body));
    assert_eq!(stub.call_count(), 1);
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(
        seen.get("litellm_call_id"),
        Some(&Json::Text(String::from("generated-1")))
    );
}

#[tokio::test]
async fn async_call_returns_the_stub_result_with_one_call_and_a_call_id() {
    let mut fixture = fixture();
    let body = value(r#"{"id":"chat-1"}"#);
    let stub = Stub::responding("acompletion", response(body.clone()));
    let outcome = fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    assert_eq!(outcome, response(body));
    assert_eq!(stub.call_count(), 1);
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(
        seen.get("litellm_call_id"),
        Some(&Json::Text(String::from("generated-1")))
    );
}

#[test]
fn sync_call_keeps_the_callers_call_id() {
    let mut fixture = fixture();
    let stub = Stub::new("completion");
    run_sync(
        &mut fixture,
        &stub,
        chat_request(r#"{"litellm_call_id":"call-7"}"#),
    )
    .expect("the call succeeds");
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(
        seen.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-7")))
    );
}

#[test]
fn sync_call_modifies_max_tokens_before_the_stub() {
    let mut fixture = fixture();
    let stub = Stub::new("completion");
    run_sync(&mut fixture, &stub, chat_request(r#"{"max_tokens":10}"#)).expect("the call succeeds");
    assert_eq!(
        fixture.tokens.calls(),
        vec![(String::from("gpt-4o"), None, 10_i64)]
    );
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(seen.get("max_tokens"), Some(&value("9")));
}

#[test]
fn sync_call_runs_post_call_and_the_success_log() {
    let mut fixture = fixture().with_cache();
    let body = value(r#"{"id":"chat-1"}"#);
    let stub = Stub::responding("completion", response(body.clone()));
    run_sync(&mut fixture, &stub, chat_request("{}")).expect("the call succeeds");
    assert_eq!(fixture.cache.stored(), vec![response(body)]);
    assert_eq!(fixture.metadata.count(), 1);
    assert_eq!(fixture.logging.events(), vec![String::from("success:chat")]);
    assert_eq!(fixture.scheduler.count(), 0);
}

#[test]
fn sync_call_runs_no_deployment_hooks() {
    let hook: Arc<dyn DeploymentHook> = Arc::new(MarkingHook::new("marker", "hooked"));
    let mut fixture = fixture().with_hooks(vec![hook]);
    let stub = Stub::new("completion");
    run_sync(&mut fixture, &stub, chat_request("{}")).expect("the call succeeds");
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(seen.get("hooked"), None);
}

#[tokio::test]
async fn async_call_runs_the_pre_call_hook_before_the_stub() {
    let hook: Arc<dyn DeploymentHook> = Arc::new(MarkingHook::new("marker", "hooked"));
    let mut fixture = fixture().with_hooks(vec![hook]);
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(seen.get("hooked"), Some(&Json::Bool(true)));
}

#[tokio::test]
async fn async_call_marks_the_response_through_the_success_hook() {
    let hook: Arc<dyn DeploymentHook> = Arc::new(MarkingHook::new("marker", "hooked"));
    let mut fixture = fixture().with_hooks(vec![hook]);
    let stub = Stub::responding("acompletion", response(value(r#"{"id":"chat-1"}"#)));
    let outcome = fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    assert_eq!(outcome, response(value(r#"{"id":"chat-1","hooked":true}"#)));
}

#[tokio::test]
async fn async_call_logs_success_in_the_background() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the call succeeds");
    assert_eq!(
        fixture.logging.events(),
        vec![String::from("sync_callbacks:chat")]
    );
    assert_eq!(fixture.scheduler.count(), 1);
    let mut jobs = fixture.scheduler.take();
    let job = jobs.remove(0);
    job.deliver().await;
    assert_eq!(
        fixture.logging.events(),
        vec![
            String::from("sync_callbacks:chat"),
            String::from("async_success:chat")
        ]
    );
}

#[test]
fn sync_async_flagged_request_takes_the_short_path() {
    let mut fixture = fixture();
    let body = value(r#"{"id":"chat-1"}"#);
    let stub = Stub::responding("completion", response(body.clone()));
    let outcome = run_sync(&mut fixture, &stub, chat_request(r#"{"acompletion":true}"#))
        .expect("the short path answers");
    assert_eq!(outcome, response(body));
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(seen.get("litellm_call_id"), None);
    assert!(stub.last_handle().is_none());
    assert!(fixture.logging.events().is_empty());
}

#[test]
fn sync_async_flagged_complete_stream_is_assembled() {
    let mut fixture = fixture();
    let stub = Stub::responding(
        "completion",
        chat_stream(value(r#"{"assembled":true}"#), false),
    );
    let request = chat_request(r#"{"acompletion":true,"stream":true,"complete_response":true}"#);
    let outcome = run_sync(&mut fixture, &stub, request).expect("the short path answers");
    assert_eq!(outcome, response(value(r#"{"assembled":true}"#)));
    assert_eq!(stub.call_count(), 1);
}

#[test]
fn sync_aspeech_flagged_request_returns_before_the_post_call_steps() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::responding("completion", response(value(r#"{"id":"chat-1"}"#)));
    let outcome = run_sync(&mut fixture, &stub, chat_request(r#"{"aspeech":true}"#))
        .expect("the call succeeds");
    assert_eq!(outcome, response(value(r#"{"id":"chat-1"}"#)));
    assert!(fixture.cache.stored().is_empty());
    assert_eq!(fixture.metadata.count(), 0);
    assert!(fixture.logging.events().is_empty());
}

#[tokio::test]
async fn async_call_drops_the_internal_call_flag() {
    let mut fixture = fixture();
    let stub = Stub::new("acompletion");
    let request = chat_request(r#"{"_is_litellm_internal_call":true}"#);
    fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect("the call succeeds");
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(seen.get("_is_litellm_internal_call"), None);
}
