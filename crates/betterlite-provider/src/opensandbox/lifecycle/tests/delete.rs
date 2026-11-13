use http::Method;
use secrecy::SecretString;

use super::super::super::helpers::{OpenSandbox, OpenSandboxError};
use super::super::{ContainerRef, LifecycleError};
use super::support::{
    ScriptedHttp, StepClock, block_on, cached_handle, delete_request, environment, header,
    plain_handle, sandbox,
};

#[test]
fn delete_sends_the_lifecycle_key_and_returns_true() {
    let sandbox = sandbox(&[(200, "")], environment(&[]));
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let deleted =
        block_on(sandbox.adelete_sandbox(delete_request(ContainerRef::Cached(&handle)), None))
            .unwrap();

    assert!(deleted);
    let request = sandbox.http.request(0);
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(
        request.url.as_str(),
        "http://stored.example/sandboxes/sbx_1"
    );
    assert_eq!(header(&request, "content-type"), Some("application/json"));
    assert_eq!(header(&request, "open-sandbox-api-key"), Some("stored-key"));
    assert!(request.body.is_empty());
    assert_eq!(request.timeout, None);
}

#[test]
fn delete_returns_false_on_404() {
    let sandbox = sandbox(
        &[(404, r#"{"code":404,"message":"gone"}"#)],
        environment(&[]),
    );
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let deleted =
        block_on(sandbox.adelete_sandbox(delete_request(ContainerRef::Cached(&handle)), None))
            .unwrap();

    assert!(!deleted);
    assert_eq!(sandbox.http.requests().len(), 1);
}

#[test]
fn delete_reports_another_error_status() {
    let sandbox = sandbox(
        &[(500, r#"{"code":500,"message":"boom"}"#)],
        environment(&[]),
    );
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let error =
        block_on(sandbox.adelete_sandbox(delete_request(ContainerRef::Cached(&handle)), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::HttpStatus { status }) if status == 500
    ));
}

#[test]
fn delete_builds_a_handle_for_a_bare_id() {
    let sandbox = sandbox(&[(204, "")], environment(&[]));
    let mut request = delete_request(ContainerRef::Id("sbx_7"));
    request.options.api_base = Some("http://sandbox.example");
    let deleted = block_on(sandbox.adelete_sandbox(request, None)).unwrap();

    assert!(deleted);
    let sent = sandbox.http.request(0);
    assert_eq!(sent.url.as_str(), "http://sandbox.example/sandboxes/sbx_7");
    assert_eq!(header(&sent, "content-type"), Some("application/json"));
    assert_eq!(header(&sent, "open-sandbox-api-key"), None);
}

#[test]
fn delete_prefers_the_stored_base_over_the_parameter() {
    let sandbox = sandbox(&[(200, "")], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let mut request = delete_request(ContainerRef::Handle(&handle));
    request.options.api_base = Some("http://override.example");
    block_on(sandbox.adelete_sandbox(request, None)).unwrap();

    assert_eq!(
        sandbox.http.request(0).url.as_str(),
        "http://stored.example/sandboxes/sbx_1"
    );
}

#[test]
fn delete_prefers_the_key_parameter_over_the_stored_key() {
    let sandbox = sandbox(&[(200, "")], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let api_key = SecretString::from("override-key");
    let mut request = delete_request(ContainerRef::Handle(&handle));
    request.options.api_key = Some(&api_key);
    block_on(sandbox.adelete_sandbox(request, None)).unwrap();

    assert_eq!(
        header(&sandbox.http.request(0), "open-sandbox-api-key"),
        Some("override-key")
    );
}

#[test]
fn delete_reads_the_key_from_the_environment() {
    let sandbox = sandbox(
        &[(200, "")],
        environment(&[("OPEN_SANDBOX_API_KEY", "env-key")]),
    );
    let handle = plain_handle(Some("http://stored.example"), None);
    block_on(sandbox.adelete_sandbox(delete_request(ContainerRef::Handle(&handle)), None)).unwrap();

    assert_eq!(
        header(&sandbox.http.request(0), "open-sandbox-api-key"),
        Some("env-key")
    );
}

#[test]
fn delete_requires_an_api_base() {
    let sandbox = sandbox(&[], environment(&[]));
    let error = block_on(sandbox.adelete_sandbox(delete_request(ContainerRef::Id("sbx_7")), None))
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::MissingApiBase { .. })
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn delete_uses_the_supplied_client_instead_of_the_pool() {
    let pool = ScriptedHttp::new(&[]);
    let caller = ScriptedHttp::new(&[(200, "")]);
    let sandbox = OpenSandbox {
        http: pool,
        environment: environment(&[]),
        clock: StepClock::new(),
    };
    let mut request = delete_request(ContainerRef::Id("sbx_7"));
    request.options.api_base = Some("http://sandbox.example");
    block_on(sandbox.adelete_sandbox(request, Some(&caller))).unwrap();

    assert!(sandbox.http.requests().is_empty());
    assert_eq!(caller.requests().len(), 1);
}
