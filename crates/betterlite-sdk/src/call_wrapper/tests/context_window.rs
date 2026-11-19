use betterlite_models::decisions::Json;

use super::support::Stub;
use super::support::chat_request;
use super::support::context_window;
use super::support::fixture;
use super::support::request;
use super::support::response;
use super::support::run_sync;
use super::support::value;
use crate::call_wrapper::state::WrapperOptions;

const MAPPED: &str = r#"{"context_window_fallback_dict":{"gpt-4o":"gpt-4o-mini"}}"#;

#[test]
fn sync_context_window_error_retries_on_the_mapped_model() {
    let mut fixture = fixture();
    let stub = Stub::scripted(
        "completion",
        vec![
            Err(context_window("too long")),
            Ok(response(value(r#"{"fallback":true}"#))),
        ],
    );
    let outcome =
        run_sync(&mut fixture, &stub, chat_request(MAPPED)).expect("the fallback model answers");
    assert_eq!(outcome, response(value(r#"{"fallback":true}"#)));
    assert_eq!(stub.call_count(), 2);
    let calls = stub.calls();
    let second = calls.get(1).expect("the target ran twice");
    assert_eq!(
        second.get("model"),
        Some(&Json::Text(String::from("gpt-4o-mini")))
    );
}

#[test]
fn sync_context_window_error_without_a_map_surfaces() {
    let mut fixture = fixture();
    let stub = Stub::failing("completion", context_window("too long"));
    let error = run_sync(&mut fixture, &stub, chat_request("{}"))
        .expect_err("no fallback map means the error surfaces");
    assert_eq!(error.to_string(), "too long");
    assert_eq!(stub.call_count(), 1);
    assert_eq!(
        fixture.logging.events(),
        vec![String::from("failure_sync:too long")]
    );
}

#[test]
fn sync_context_window_model_outside_the_map_surfaces() {
    let mut fixture = fixture();
    let stub = Stub::failing("completion", context_window("too long"));
    let request = chat_request(r#"{"context_window_fallback_dict":{"other":"gpt-4o-mini"}}"#);
    let error =
        run_sync(&mut fixture, &stub, request).expect_err("an unmapped model has no fallback");
    assert_eq!(error.to_string(), "too long");
    assert_eq!(stub.call_count(), 1);
}

#[tokio::test]
async fn async_context_window_error_retries_on_the_mapped_model() {
    let mut fixture = fixture();
    let stub = Stub::scripted(
        "acompletion",
        vec![
            Err(context_window("too long")),
            Ok(response(value(r#"{"fallback":true}"#))),
        ],
    );
    let outcome = fixture
        .wrapper
        .call(&stub, chat_request(MAPPED), WrapperOptions::default())
        .await
        .expect("the fallback model answers");
    assert_eq!(outcome, response(value(r#"{"fallback":true}"#)));
    let calls = stub.calls();
    let second = calls.get(1).expect("the target ran twice");
    assert_eq!(
        second.get("model"),
        Some(&Json::Text(String::from("gpt-4o-mini")))
    );
    assert_eq!(stub.retry_count(), 0);
}

#[tokio::test]
async fn aresponses_context_window_error_does_not_fall_back() {
    let mut fixture = fixture();
    let stub = Stub::failing("aresponses", context_window("too long"));
    let request = request(
        r#"{"model":"gpt-4o","input":"hi","context_window_fallback_dict":{"gpt-4o":"gpt-4o-mini"}}"#,
    );
    let error = fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect_err("the responses path has no context fallback");
    assert_eq!(error.to_string(), "too long");
    assert_eq!(stub.call_count(), 1);
}
