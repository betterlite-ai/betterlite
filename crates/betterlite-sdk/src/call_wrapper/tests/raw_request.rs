use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::Stub;
use super::support::fixture;
use super::support::plain;
use super::support::request;
use super::support::value;
use crate::call_wrapper::error::CallErrorKind;
use crate::call_wrapper::raw_request::RAW_REQUEST_CAPTURED;

fn error_object(message: &str) -> Json {
    Json::Object(IndexMap::from([(
        "error".to_owned(),
        Json::Text(message.to_owned()),
    )]))
}

#[tokio::test]
async fn raw_request_returns_the_captured_dict() {
    let fixture = fixture();
    let captured = value(r#"{"model":"gpt-3.5-turbo","messages":[]}"#);
    let stub = Stub::new("completion").with_capture(captured.clone());
    let source = request(r#"{"model":"gpt-4o","api_key":"real-key","messages":[]}"#);
    let returned = fixture.wrapper.return_raw_request(&stub, source).await;
    assert_eq!(returned, captured);
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw the raw request");
    assert_eq!(
        seen.get("api_key"),
        Some(&Json::Text(String::from("my-fake-api-key")))
    );
}

#[tokio::test]
async fn raw_request_with_a_non_object_capture_returns_the_error_form() {
    let fixture = fixture();
    let stub = Stub::new("completion").with_capture(Json::Text(String::from("captured")));
    let source = request(r#"{"model":"gpt-4o","messages":[]}"#);
    let returned = fixture.wrapper.return_raw_request(&stub, source).await;
    assert_eq!(returned, error_object(""));
}

#[tokio::test]
async fn raw_request_returns_an_empty_error_when_nothing_was_captured() {
    let fixture = fixture();
    let stub = Stub::new("completion");
    let source = request(r#"{"model":"gpt-4o","messages":[]}"#);
    let returned = fixture.wrapper.return_raw_request(&stub, source).await;
    assert_eq!(returned, error_object(""));
}

#[tokio::test]
async fn raw_request_returns_the_failure_text() {
    let fixture = fixture();
    let stub = Stub::failing("completion", plain("boom"));
    let source = request(r#"{"model":"gpt-4o","messages":[]}"#);
    let returned = fixture.wrapper.return_raw_request(&stub, source).await;
    assert_eq!(returned, error_object("boom"));
}

#[tokio::test]
async fn raw_request_reports_the_capture_sentinel() {
    let fixture = fixture();
    let mut error = plain("ignored");
    error.kind = CallErrorKind::RawRequestCaptured;
    let stub = Stub::failing("completion", error);
    let source = request(r#"{"model":"gpt-4o","messages":[]}"#);
    let returned = fixture.wrapper.return_raw_request(&stub, source).await;
    assert_eq!(returned, error_object(RAW_REQUEST_CAPTURED));
}
