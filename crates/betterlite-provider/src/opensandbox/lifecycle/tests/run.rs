use std::time::Duration;

use http::Method;
use secrecy::SecretString;

use super::super::super::helpers::{OpenSandboxError, RequestTimeout};
use super::super::{ContainerRef, LifecycleError};
use super::support::{
    block_on, cached_handle, environment, header, plain_handle, run_request, sandbox,
};

const ENDPOINT: &str =
    r#"{"endpoint":"http://execd.local/","headers":{"X-Execd-Token":"execd-token"}}"#;
const OUTPUT: &str = concat!(
    r#"data: {"type":"stdout","text":"hello\n"}"#,
    "\n",
    r#"data: {"type":"execution_count","execution_count":1}"#,
    "\n",
);
const NOT_FOUND: &str = r#"{"code":404,"message":"endpoint not ready"}"#;

#[test]
fn run_uses_the_cached_endpoint_and_only_the_endpoint_headers() {
    let sandbox = sandbox(&[(200, OUTPUT)], environment(&[]));
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let result = block_on(sandbox.arun_code(
        run_request(ContainerRef::Cached(&handle), "print('hello')"),
        None,
    ))
    .unwrap();

    assert_eq!(result.stdout, "hello\n");
    assert_eq!(result.execution_count, Some(1));
    assert_eq!(result.object, "code_execution");
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.as_str(), "http://execd.local/code");
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/json")
    );
    assert_eq!(header(&requests[0], "accept"), Some("text/event-stream"));
    assert_eq!(header(&requests[0], "cache-control"), Some("no-cache"));
    assert_eq!(header(&requests[0], "x-execd-token"), Some("execd-token"));
    assert_eq!(header(&requests[0], "open-sandbox-api-key"), None);
    assert_eq!(
        String::from_utf8(requests[0].body.clone()).unwrap(),
        r#"{"code":"print('hello')","context":{"language":"python"}}"#
    );
    assert_eq!(
        requests[0].timeout,
        Some(RequestTimeout {
            connect: Some(Duration::from_secs(30)),
            read: None,
            write: Some(Duration::from_secs(30)),
            pool: None,
        })
    );
}

#[test]
fn run_reacquires_the_endpoint_for_a_bare_id() {
    let sandbox = sandbox(&[(200, ENDPOINT), (200, OUTPUT)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let mut request = run_request(ContainerRef::Id("sbx_1"), "1 + 1");
    request.api_base = Some("http://sandbox.example");
    request.api_key = Some(&api_key);
    let result = block_on(sandbox.arun_code(request, None)).unwrap();

    assert_eq!(result.stdout, "hello\n");
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
    assert_eq!(header(&requests[0], "open-sandbox-api-key"), Some("key-1"));
    assert_eq!(requests[1].url.as_str(), "http://execd.local/code");
    assert_eq!(header(&requests[1], "open-sandbox-api-key"), None);
}

#[test]
fn run_reacquires_the_endpoint_for_a_plain_handle() {
    let sandbox = sandbox(&[(200, ENDPOINT), (200, OUTPUT)], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let result =
        block_on(sandbox.arun_code(run_request(ContainerRef::Handle(&handle), "1"), None)).unwrap();

    assert_eq!(result.stdout, "hello\n");
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url.as_str(),
        "http://stored.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
    assert_eq!(
        header(&requests[0], "open-sandbox-api-key"),
        Some("stored-key")
    );
}

#[test]
fn run_prefers_the_stored_base_and_the_explicit_key() {
    let sandbox = sandbox(&[(200, ENDPOINT), (200, OUTPUT)], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let api_key = SecretString::from("override-key");
    let mut request = run_request(ContainerRef::Handle(&handle), "1");
    request.api_base = Some("http://override.example");
    request.api_key = Some(&api_key);
    block_on(sandbox.arun_code(request, None)).unwrap();

    let requests = sandbox.http.requests();
    assert_eq!(
        requests[0].url.as_str(),
        "http://stored.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
    assert_eq!(
        header(&requests[0], "open-sandbox-api-key"),
        Some("override-key")
    );
}

#[test]
fn run_passes_a_custom_language() {
    let sandbox = sandbox(&[(200, OUTPUT)], environment(&[]));
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let mut request = run_request(ContainerRef::Cached(&handle), "print(1)");
    request.language = Some("r");
    block_on(sandbox.arun_code(request, None)).unwrap();

    assert_eq!(
        String::from_utf8(sandbox.http.request(0).body).unwrap(),
        r#"{"code":"print(1)","context":{"language":"r"}}"#
    );
}

#[test]
fn run_requires_an_api_base() {
    let sandbox = sandbox(&[], environment(&[]));
    let error =
        block_on(sandbox.arun_code(run_request(ContainerRef::Id("sbx_1"), "1"), None)).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::MissingApiBase { variable })
            if variable == "OPEN_SANDBOX_API_BASE"
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn run_reports_a_non_success_status() {
    let sandbox = sandbox(
        &[(200, ENDPOINT), (500, r#"{"code":500,"message":"boom"}"#)],
        environment(&[]),
    );
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let error = block_on(sandbox.arun_code(run_request(ContainerRef::Handle(&handle), "1"), None))
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::HttpStatus { status }) if status == 500
    ));
}

#[test]
fn run_rejects_an_endpoint_header_name_a_request_cannot_carry() {
    let sandbox = sandbox(&[], environment(&[]));
    let mut handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    handle
        .execd_endpoint
        .headers
        .insert("bad header".to_owned(), "value".to_owned());
    let error = block_on(sandbox.arun_code(run_request(ContainerRef::Cached(&handle), "1"), None))
        .unwrap_err();

    assert!(matches!(error, LifecycleError::InvalidHeaderName { .. }));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn run_rejects_an_endpoint_header_value_a_request_cannot_carry() {
    let sandbox = sandbox(&[], environment(&[]));
    let mut handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    handle
        .execd_endpoint
        .headers
        .insert("X-Bad".to_owned(), "bad\nvalue".to_owned());
    let error = block_on(sandbox.arun_code(run_request(ContainerRef::Cached(&handle), "1"), None))
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::InvalidHeader { .. })
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn run_reports_an_endpoint_that_never_arrives() {
    let sandbox = sandbox(&[(404, NOT_FOUND)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let mut request = run_request(ContainerRef::Id("sbx_1"), "1");
    request.api_base = Some("http://sandbox.example");
    request.api_key = Some(&api_key);
    request.ready_timeout = Some(Duration::from_secs(1));
    request.poll_interval = Some(Duration::from_millis(500));
    let error = block_on(sandbox.arun_code(request, None)).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::EndpointNotReady { timeout, .. })
            if timeout == Duration::from_secs(1)
    ));
    assert_eq!(sandbox.http.requests().len(), 3);
}

#[test]
fn run_forwards_the_server_proxy_flag_when_it_reacquires_the_endpoint() {
    let sandbox = sandbox(&[(200, ENDPOINT), (200, OUTPUT)], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let mut request = run_request(ContainerRef::Handle(&handle), "1");
    request.use_server_proxy = true;
    block_on(sandbox.arun_code(request, None)).unwrap();

    assert_eq!(
        sandbox.http.request(0).url.as_str(),
        "http://stored.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=true"
    );
}
