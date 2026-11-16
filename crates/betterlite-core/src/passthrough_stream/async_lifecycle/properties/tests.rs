#![cfg(test)]

use betterlite_models::decisions::Json;
use http::HeaderValue;

use crate::passthrough_stream::error::PassthroughStreamError;
use crate::passthrough_stream::tests::{Fixture, ReplyFixture, script};

#[test]
fn status_code_before_init_names_the_field() {
    let fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    let error = fixture.stream.status_code().expect_err("not initialized");
    assert_eq!(
        error.to_string(),
        "AsyncPassthroughStream must be awaited before accessing status_code"
    );
    let PassthroughStreamError::NotInitialized { field } = error else {
        panic!("expected a not-initialized error");
    };
    assert_eq!(field, "status_code");
}

#[test]
fn headers_before_init_names_the_field() {
    let fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    let error = fixture.stream.headers().expect_err("not initialized");
    assert_eq!(
        error.to_string(),
        "AsyncPassthroughStream must be awaited before accessing headers"
    );
    let PassthroughStreamError::NotInitialized { field } = error else {
        panic!("expected a not-initialized error");
    };
    assert_eq!(field, "headers");
}

#[tokio::test]
async fn accessors_report_the_initialized_response() {
    let mut fixture = Fixture::ready(ReplyFixture::ok(script(Vec::new()))).await;
    assert_eq!(fixture.stream.status_code().expect("initialized"), 200);
    assert_eq!(
        fixture
            .stream
            .headers()
            .expect("initialized")
            .get("x-upstream"),
        Some(&HeaderValue::from_static("yes"))
    );
}

#[test]
fn fields_start_unset_before_init() {
    let fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    assert!(!fixture.stream.initialized);
    assert_eq!(fixture.stream.status_code, 0);
    assert!(fixture.stream.headers.is_empty());
    assert!(fixture.stream.hidden_params.is_empty());
    assert!(fixture.stream.pending.is_some());
    assert!(fixture.stream.source.is_none());
    assert!(!fixture.stream.flush_scheduled);
    assert!(fixture.tracker.is_empty());
}

#[test]
fn hidden_params_accepts_router_attached_headers() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    fixture
        .stream
        .hidden_params
        .insert("x-request-id".to_owned(), Json::Text("req-1".to_owned()));
    assert_eq!(
        fixture.stream.hidden_params.get("x-request-id"),
        Some(&Json::Text("req-1".to_owned()))
    );
}
