use std::time::Duration;

use betterlite_models::sandbox::SANDBOX_MAX_OUTPUT_BYTES;
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, Method};
use rstest::rstest;

use super::super::OpenSandboxError;
use super::super::request::{ExecutionContext, PostCode, PostCodeBody};
use super::{FakeHttp, block_on, environment, sandbox};

fn post_body() -> PostCodeBody {
    PostCodeBody {
        code: "print(1)".to_owned(),
        context: ExecutionContext {
            language: "python".to_owned(),
        },
    }
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

#[test]
fn post_code_sends_the_execution_request_and_returns_every_line() {
    let sandbox = sandbox(
        FakeHttp::new(&[(200, "first\nsecond\r\nthird")]),
        environment(&[]),
    );
    let body = post_body();
    let lines = block_on(sandbox.post_code(
        PostCode {
            url: "http://sandbox.example/code",
            headers: headers(),
            body: &body,
        },
        None,
    ))
    .unwrap();
    assert_eq!(lines, vec!["first", "second", "third"]);
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "http://sandbox.example/code");
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(
        std::str::from_utf8(&request.body).unwrap(),
        r#"{"code":"print(1)","context":{"language":"python"}}"#
    );
    let timeout = request.timeout.unwrap();
    assert_eq!(timeout.connect, Some(Duration::from_secs(30)));
    assert_eq!(timeout.read, None);
    assert_eq!(timeout.write, Some(Duration::from_secs(30)));
    assert_eq!(timeout.pool, None);
}

#[rstest]
#[case::client_error(400)]
#[case::not_found(404)]
#[case::server_error(500)]
fn post_code_reports_a_non_success_status(#[case] status: u16) {
    let sandbox = sandbox(FakeHttp::new(&[(status, "ignored")]), environment(&[]));
    let body = post_body();
    let error = block_on(sandbox.post_code(
        PostCode {
            url: "http://sandbox.example/code",
            headers: headers(),
            body: &body,
        },
        None,
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        OpenSandboxError::HttpStatus { status: observed } if observed == status
    ));
}

#[test]
fn post_code_stops_at_the_output_cap() {
    let body = "x".repeat(SANDBOX_MAX_OUTPUT_BYTES.saturating_add(1));
    let sandbox = sandbox(FakeHttp::new(&[(200, body.as_str())]), environment(&[]));
    let post = post_body();
    let error = block_on(sandbox.post_code(
        PostCode {
            url: "http://sandbox.example/code",
            headers: headers(),
            body: &post,
        },
        None,
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        OpenSandboxError::OutputTooLarge { limit } if limit == SANDBOX_MAX_OUTPUT_BYTES
    ));
}

#[test]
fn post_code_uses_a_supplied_client_over_the_injected_one() {
    let supplied = FakeHttp::new(&[(200, "line")]);
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[]));
    let body = post_body();
    let lines = block_on(sandbox.post_code(
        PostCode {
            url: "http://sandbox.example/code",
            headers: headers(),
            body: &body,
        },
        Some(&supplied),
    ))
    .unwrap();
    assert_eq!(lines, vec!["line"]);
    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(sandbox.http.requests().len(), 0);
}
