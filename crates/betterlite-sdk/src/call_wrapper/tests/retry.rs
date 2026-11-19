use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::support::Stub;
use super::support::api_error;
use super::support::bad_request;
use super::support::chat_request;
use super::support::fixture;
use super::support::plain;
use super::support::rate_limit;
use super::support::request;
use super::support::response;
use super::support::run_sync;
use super::support::value;
use crate::call_wrapper::state::WrapperOptions;

fn count(value: i64) -> Json {
    Json::Number(Number::from(value))
}

#[test]
fn sync_retry_limit_error_comes_before_the_stub() {
    let mut fixture = fixture();
    fixture.wrapper.retry.num_retries_per_request = Some(2);
    let stub = Stub::new("completion");
    let request = chat_request(r#"{"metadata":{"request_retry_count":2}}"#);
    let error =
        run_sync(&mut fixture, &stub, request).expect_err("the per-request limit stops the call");
    assert_eq!(error.to_string(), "Max retries per request hit!");
    assert_eq!(stub.call_count(), 0);
}

#[tokio::test]
async fn async_retry_limit_is_not_checked() {
    let mut fixture = fixture();
    fixture.wrapper.retry.num_retries_per_request = Some(2);
    let stub = Stub::new("acompletion");
    let request = chat_request(r#"{"metadata":{"request_retry_count":2}}"#);
    fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect("the async path has no retry limit");
    assert_eq!(stub.call_count(), 1);
}

#[test]
fn sync_failure_retries_with_the_request_count() {
    let mut fixture = fixture();
    let stub = Stub::failing("completion", api_error("boom"))
        .with_retry(response(value(r#"{"retried":true}"#)));
    let outcome = run_sync(&mut fixture, &stub, chat_request(r#"{"num_retries":2}"#))
        .expect("the retry entry answers");
    assert_eq!(outcome, response(value(r#"{"retried":true}"#)));
    assert_eq!(stub.retry_count(), 1);
    let retries = stub.retries();
    let retried = retries.first().expect("the retry entry saw one request");
    assert_eq!(retried.get("num_retries"), Some(&count(2)));
    assert!(fixture.logging.events().is_empty());
}

#[test]
fn sync_retry_policy_overrides_the_request_count_and_is_reset() {
    let mut fixture = fixture();
    let stub = Stub::failing("completion", bad_request("bad"))
        .with_retry(response(value(r#"{"retried":true}"#)));
    let request = chat_request(r#"{"num_retries":5,"retry_policy":{"BadRequestErrorRetries":1}}"#);
    run_sync(&mut fixture, &stub, request).expect("the retry entry answers");
    let retries = stub.retries();
    let retried = retries.first().expect("the retry entry saw one request");
    assert_eq!(retried.get("num_retries"), Some(&count(1)));
    assert_eq!(
        retried.get("retry_policy"),
        Some(&Json::Object(IndexMap::new()))
    );
}

#[test]
fn sync_router_call_is_not_retried_and_the_default_is_cleared() {
    let mut fixture = fixture();
    fixture.wrapper.retry.num_retries = Some(2);
    let stub = Stub::failing("completion", api_error("boom")).with_retry(response(value("{}")));
    let request = chat_request(r#"{"metadata":{"model_group":"gpt-4"}}"#);
    let error = run_sync(&mut fixture, &stub, request).expect_err("the router call is not retried");
    assert_eq!(error.to_string(), "boom");
    assert_eq!(stub.retry_count(), 0);
    assert_eq!(fixture.wrapper.retry.num_retries, None);
}

#[test]
fn sync_non_api_error_is_not_retried() {
    let mut fixture = fixture();
    let stub = Stub::failing("completion", plain("plain")).with_retry(response(value("{}")));
    let error = run_sync(&mut fixture, &stub, chat_request(r#"{"num_retries":2}"#))
        .expect_err("a plain exception does not retry");
    assert_eq!(error.to_string(), "plain");
    assert_eq!(stub.retry_count(), 0);
    assert_eq!(
        fixture.logging.events(),
        vec![String::from("failure_sync:plain")]
    );
}

#[test]
fn sync_retry_default_is_cleared_after_use() {
    let mut fixture = fixture();
    fixture.wrapper.retry.num_retries = Some(1);
    let stub = Stub::failing("completion", api_error("boom"))
        .with_retry(response(value(r#"{"retried":true}"#)));
    run_sync(&mut fixture, &stub, chat_request("{}")).expect("the retry entry answers");
    assert_eq!(stub.retry_count(), 1);
    assert_eq!(fixture.wrapper.retry.num_retries, None);
}

#[test]
fn sync_embedding_call_is_not_retried() {
    let mut fixture = fixture();
    let stub = Stub::failing("embedding", api_error("boom")).with_retry(response(value("{}")));
    let request = request(r#"{"model":"text-embedding-3-small","input":"hi","num_retries":2}"#);
    let error = run_sync(&mut fixture, &stub, request).expect_err("only chat calls retry here");
    assert_eq!(error.to_string(), "boom");
    assert_eq!(stub.retry_count(), 0);
}

#[test]
fn sync_responses_call_retries_through_the_retry_entry() {
    let mut fixture = fixture();
    let stub = Stub::failing("responses", api_error("boom"))
        .with_retry(response(value(r#"{"retried":true}"#)));
    let request = request(r#"{"model":"gpt-4o","input":"hi","num_retries":1}"#);
    let outcome = run_sync(&mut fixture, &stub, request).expect("the retry entry answers");
    assert_eq!(outcome, response(value(r#"{"retried":true}"#)));
    assert_eq!(stub.retry_count(), 1);
}

#[tokio::test]
async fn async_responses_call_retries_through_the_retry_entry() {
    let mut fixture = fixture();
    let stub = Stub::failing("aresponses", api_error("boom"))
        .with_retry(response(value(r#"{"retried":true}"#)));
    let request = request(r#"{"model":"gpt-4o","input":"hi","num_retries":1}"#);
    let outcome = fixture
        .wrapper
        .call(&stub, request, WrapperOptions::default())
        .await
        .expect("the retry entry answers");
    assert_eq!(outcome, response(value(r#"{"retried":true}"#)));
    assert_eq!(stub.retry_count(), 1);
}

#[tokio::test]
async fn async_failure_retries_and_swallows_a_failed_retry() {
    let mut fixture = fixture();
    let stub = Stub::failing("acompletion", api_error("boom")).with_failed_retry(plain("broken"));
    let error = fixture
        .wrapper
        .call(
            &stub,
            chat_request(r#"{"num_retries":1}"#),
            WrapperOptions::default(),
        )
        .await
        .expect_err("the original error wins");
    assert_eq!(error.to_string(), "boom");
    assert_eq!(stub.retry_count(), 1);
    assert_eq!(
        fixture.logging.events(),
        vec![
            String::from("failure_sync:boom"),
            String::from("failure_async:boom")
        ]
    );
}

#[tokio::test]
async fn async_retry_stamps_exponential_for_rate_limits() {
    let mut fixture = fixture();
    let stub = Stub::failing("acompletion", rate_limit("slow")).with_retry(response(value("{}")));
    fixture
        .wrapper
        .call(
            &stub,
            chat_request(r#"{"num_retries":1}"#),
            WrapperOptions::default(),
        )
        .await
        .expect("the retry entry answers");
    let retries = stub.retries();
    let retried = retries.first().expect("the retry entry saw one request");
    assert_eq!(
        retried.get("retry_strategy"),
        Some(&Json::Text(String::from("exponential_backoff_retry")))
    );
}

#[tokio::test]
async fn async_retry_stamps_constant_for_api_errors() {
    let mut fixture = fixture();
    let stub = Stub::failing("acompletion", api_error("boom")).with_retry(response(value("{}")));
    fixture
        .wrapper
        .call(
            &stub,
            chat_request(r#"{"num_retries":1}"#),
            WrapperOptions::default(),
        )
        .await
        .expect("the retry entry answers");
    let retries = stub.retries();
    let retried = retries.first().expect("the retry entry saw one request");
    assert_eq!(
        retried.get("retry_strategy"),
        Some(&Json::Text(String::from("constant_retry")))
    );
}

#[tokio::test]
async fn async_retry_of_a_plain_error_has_no_strategy() {
    let mut fixture = fixture();
    let stub = Stub::failing("acompletion", plain("plain")).with_retry(response(value("{}")));
    fixture
        .wrapper
        .call(
            &stub,
            chat_request(r#"{"num_retries":1}"#),
            WrapperOptions::default(),
        )
        .await
        .expect("the retry entry answers");
    assert_eq!(stub.retry_count(), 1);
    let retries = stub.retries();
    let retried = retries.first().expect("the retry entry saw one request");
    assert_eq!(retried.get("retry_strategy"), None);
}

#[tokio::test]
async fn async_failure_stamps_the_request_count_and_timeout() {
    let mut fixture = fixture();
    let stub = Stub::failing("acompletion", api_error("boom"));
    let error = fixture
        .wrapper
        .call(
            &stub,
            chat_request(r#"{"num_retries":0,"timeout":30}"#),
            WrapperOptions::default(),
        )
        .await
        .expect_err("no retry is left");
    assert_eq!(error.to_string(), "boom");
    assert_eq!(error.num_retries, Some(0));
    assert_eq!(error.timeout, Some(count(30)));
    assert_eq!(stub.retry_count(), 0);
}
