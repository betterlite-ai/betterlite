use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use super::super::super::helpers::OpenSandboxError;
use super::super::{ContainerRef, LifecycleError};
use super::support::{
    block_on, cached_handle, environment, header, plain_handle, run_request, sandbox,
};

const ENDPOINT: &str = r#"{"endpoint":"http://execd.local","headers":{}}"#;
const NOT_FOUND: &str = r#"{"code":404,"message":"endpoint not ready"}"#;

#[test]
fn ensure_returns_a_cached_handle_without_asking_the_api() {
    let sandbox = sandbox(&[], environment(&[]));
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let resolved =
        block_on(sandbox.ensure_handle(&run_request(ContainerRef::Cached(&handle), "1"), None))
            .unwrap();

    assert!(sandbox.http.requests().is_empty());
    assert_eq!(resolved.container.id, "sbx_1");
    assert_eq!(
        resolved.container.hidden_params.api_base.as_deref(),
        Some("http://stored.example")
    );
    assert_eq!(resolved.execd_endpoint, "http://execd.local");
    assert!(resolved.use_server_proxy);
}

#[test]
fn ensure_reacquires_the_endpoint_and_records_the_resolved_base() {
    let sandbox = sandbox(&[(200, ENDPOINT)], environment(&[]));
    let handle = plain_handle(None, None);
    let api_key = SecretString::from("explicit-key");
    let mut request = run_request(ContainerRef::Handle(&handle), "1");
    request.options.api_base = Some("http://sandbox.example");
    request.options.api_key = Some(&api_key);
    request.options.use_server_proxy = true;
    let resolved = block_on(sandbox.ensure_handle(&request, None)).unwrap();

    assert_eq!(
        resolved.container.domain.as_deref(),
        Some("http://sandbox.example")
    );
    assert_eq!(
        resolved.container.hidden_params.api_base.as_deref(),
        Some("http://sandbox.example")
    );
    assert_eq!(
        resolved
            .container
            .hidden_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("explicit-key".to_owned())
    );
    assert!(resolved.use_server_proxy);
    assert_eq!(
        sandbox.http.request(0).url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=true"
    );
}

#[test]
fn ensure_keeps_the_access_tokens_the_handle_already_carried() {
    let sandbox = sandbox(&[(200, ENDPOINT)], environment(&[]));
    let mut handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    handle.hidden_params.envd_access_token = Some(SecretString::from("envd-token"));
    handle.hidden_params.traffic_access_token = Some(SecretString::from("traffic-token"));
    let resolved =
        block_on(sandbox.ensure_handle(&run_request(ContainerRef::Handle(&handle), "1"), None))
            .unwrap();

    assert_eq!(
        resolved
            .container
            .hidden_params
            .envd_access_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("envd-token".to_owned())
    );
    assert_eq!(
        resolved
            .container
            .hidden_params
            .traffic_access_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("traffic-token".to_owned())
    );
}

#[test]
fn ensure_prefers_the_stored_base_and_the_stored_key() {
    let sandbox = sandbox(&[(200, ENDPOINT)], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let mut request = run_request(ContainerRef::Handle(&handle), "1");
    request.options.api_base = Some("http://override.example");
    let resolved = block_on(sandbox.ensure_handle(&request, None)).unwrap();

    assert_eq!(
        resolved.container.hidden_params.api_base.as_deref(),
        Some("http://stored.example")
    );
    assert_eq!(
        sandbox.http.request(0).url.as_str(),
        "http://stored.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
    assert_eq!(
        header(&sandbox.http.request(0), "open-sandbox-api-key"),
        Some("stored-key")
    );
}

#[test]
fn ensure_replaces_a_stale_domain_with_the_resolved_api_base() {
    let sandbox = sandbox(&[(200, ENDPOINT)], environment(&[]));
    let mut handle = plain_handle(None, None);
    handle.domain = Some("http://stale.example".to_owned());
    let mut request = run_request(ContainerRef::Handle(&handle), "1");
    request.options.api_base = Some("http://fresh.example");
    let resolved = block_on(sandbox.ensure_handle(&request, None)).unwrap();

    assert_eq!(
        resolved.container.domain.as_deref(),
        Some("http://fresh.example")
    );
    assert_eq!(
        resolved.container.hidden_params.api_base.as_deref(),
        Some("http://fresh.example")
    );
}

#[test]
fn ensure_reads_the_key_from_the_environment() {
    let sandbox = sandbox(
        &[(200, ENDPOINT)],
        environment(&[("OPEN_SANDBOX_API_KEY", "env-key")]),
    );
    let handle = plain_handle(Some("http://stored.example"), None);
    block_on(sandbox.ensure_handle(&run_request(ContainerRef::Handle(&handle), "1"), None))
        .unwrap();

    assert_eq!(
        header(&sandbox.http.request(0), "open-sandbox-api-key"),
        Some("env-key")
    );
}

#[test]
fn ensure_requires_an_api_base() {
    let sandbox = sandbox(&[], environment(&[]));
    let handle = plain_handle(None, Some("stored-key"));
    let error =
        block_on(sandbox.ensure_handle(&run_request(ContainerRef::Handle(&handle), "1"), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::MissingApiBase { .. })
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn ensure_reports_an_endpoint_that_never_arrives() {
    let sandbox = sandbox(&[(404, NOT_FOUND)], environment(&[]));
    let handle = plain_handle(Some("http://stored.example"), Some("stored-key"));
    let mut request = run_request(ContainerRef::Handle(&handle), "1");
    request.options.ready_timeout = Some(Duration::from_secs(1));
    request.options.poll_interval = Some(Duration::from_millis(500));
    let error = block_on(sandbox.ensure_handle(&request, None)).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::EndpointNotReady { timeout, .. })
            if timeout == Duration::from_secs(1)
    ));
    assert_eq!(sandbox.http.requests().len(), 3);
}
