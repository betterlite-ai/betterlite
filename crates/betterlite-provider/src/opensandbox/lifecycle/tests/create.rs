use std::collections::BTreeMap;
use std::time::Duration;

use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::super::super::helpers::{OpenSandbox, OpenSandboxError};
use super::super::LifecycleError;
use super::support::{
    ScriptedHttp, StepClock, block_on, create_request, environment, header, sandbox,
};

const RUNNING: &str = r#"{"id":"sbx_1","status":{"state":"Running"}}"#;
const PENDING: &str = r#"{"id":"sbx_1","status":{"state":"Pending"}}"#;
const ENDPOINT: &str =
    r#"{"endpoint":"http://execd.local/","headers":{"X-Execd-Token":"token-1"}}"#;
const NOT_FOUND: &str = r#"{"code":404,"message":"endpoint not ready"}"#;
const DEFAULT_BODY: &str = concat!(
    r#"{"image":{"uri":"opensandbox/code-interpreter:v1.1.0"},"#,
    r#""entrypoint":["/opt/code-interpreter/code-interpreter.sh"],"#,
    r#""timeout":300,"resourceLimits":{"cpu":"1","memory":"2Gi"},"#,
    r#""networkPolicy":{"defaultAction":"deny","egress":[]}}"#
);

#[test]
fn create_posts_the_default_body_and_the_lifecycle_headers() {
    let sandbox = sandbox(&[(200, RUNNING), (200, ENDPOINT)], environment(&[]));
    let api_key = SecretString::from("key-1");
    block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example/", &api_key), None))
        .unwrap();

    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.as_str(), "http://sandbox.example/sandboxes");
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/json")
    );
    assert_eq!(header(&requests[0], "open-sandbox-api-key"), Some("key-1"));
    assert_eq!(requests[0].timeout, None);
    assert_eq!(requests[0].body, DEFAULT_BODY.as_bytes());
    assert_eq!(requests[1].method, Method::GET);
    assert_eq!(
        requests[1].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
    assert_eq!(header(&requests[1], "open-sandbox-api-key"), Some("key-1"));
}

#[test]
fn create_caches_the_endpoint_and_the_credentials_in_the_handle() {
    let sandbox = sandbox(&[(200, RUNNING), (200, ENDPOINT)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let handle = block_on(
        sandbox.acreate_sandbox(create_request("http://sandbox.example/", &api_key), None),
    )
    .unwrap();

    assert_eq!(handle.container.id, "sbx_1");
    assert_eq!(handle.container.provider, "opensandbox");
    assert_eq!(
        handle.container.domain.as_deref(),
        Some("http://sandbox.example")
    );
    assert_eq!(
        handle.container.hidden_params.api_base.as_deref(),
        Some("http://sandbox.example")
    );
    assert_eq!(
        handle
            .container
            .hidden_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("key-1".to_owned())
    );
    assert_eq!(handle.execd_endpoint, "http://execd.local/");
    assert_eq!(
        handle
            .execd_headers
            .get("X-Execd-Token")
            .map(|token| token.expose_secret().to_owned()),
        Some("token-1".to_owned())
    );
    assert!(!handle.use_server_proxy);
}

#[test]
fn create_waits_for_running_before_asking_for_the_endpoint() {
    let sandbox = sandbox(
        &[
            (200, PENDING),
            (200, PENDING),
            (200, RUNNING),
            (200, ENDPOINT),
        ],
        environment(&[]),
    );
    let api_key = SecretString::from("key-1");
    block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
        .unwrap();

    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[1].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1"
    );
    assert_eq!(
        requests[2].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1"
    );
    assert_eq!(
        requests[3].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
}

#[test]
fn create_polls_the_endpoint_until_it_answers() {
    let sandbox = sandbox(
        &[(200, RUNNING), (404, NOT_FOUND), (200, ENDPOINT)],
        environment(&[]),
    );
    let api_key = SecretString::from("key-1");
    let mut request = create_request("http://sandbox.example", &api_key);
    request.options.poll_interval = Some(Duration::from_millis(10));
    let handle = block_on(sandbox.acreate_sandbox(request, None)).unwrap();

    assert_eq!(handle.execd_endpoint, "http://execd.local/");
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
    assert_eq!(
        requests[2].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
}

#[test]
fn create_defaults_to_a_thirty_second_ready_timeout_and_a_200ms_poll() {
    let sandbox = sandbox(&[(200, RUNNING), (404, NOT_FOUND)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let error =
        block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::EndpointNotReady { timeout, .. })
            if timeout == Duration::from_secs(30)
    ));
    assert_eq!(sandbox.http.requests().len(), 152);
    let sleeps = sandbox.clock.sleeps();
    assert_eq!(sleeps.len(), 150);
    assert!(
        sleeps
            .iter()
            .all(|sleep| *sleep == Duration::from_millis(200))
    );
}

#[test]
fn create_uses_the_given_ready_timeout_and_poll_interval() {
    let sandbox = sandbox(&[(200, RUNNING), (404, NOT_FOUND)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let mut request = create_request("http://sandbox.example", &api_key);
    request.options.ready_timeout = Some(Duration::from_secs(1));
    request.options.poll_interval = Some(Duration::from_millis(500));
    let error = block_on(sandbox.acreate_sandbox(request, None)).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::EndpointNotReady { timeout, .. })
            if timeout == Duration::from_secs(1)
    ));
    assert_eq!(sandbox.http.requests().len(), 4);
    assert_eq!(sandbox.clock.sleeps(), vec![Duration::from_millis(500); 2]);
}

#[test]
fn create_stops_when_the_sandbox_enters_a_terminal_state() {
    let sandbox = sandbox(
        &[
            (200, PENDING),
            (200, r#"{"id":"sbx_1","status":{"state":"Failed"}}"#),
        ],
        environment(&[]),
    );
    let api_key = SecretString::from("key-1");
    let error =
        block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::UnexpectedState { sandbox_id, state })
            if sandbox_id == "sbx_1" && state == "Failed"
    ));
    assert_eq!(sandbox.http.requests().len(), 2);
}

#[test]
fn create_reports_a_sandbox_that_never_runs() {
    let sandbox = sandbox(&[(200, PENDING)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let mut request = create_request("http://sandbox.example", &api_key);
    request.options.ready_timeout = Some(Duration::from_secs(1));
    request.options.poll_interval = Some(Duration::from_millis(500));
    let error = block_on(sandbox.acreate_sandbox(request, None)).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::NotRunning { timeout, sandbox_id })
            if timeout == Duration::from_secs(1) && sandbox_id == "sbx_1"
    ));
    assert_eq!(sandbox.http.requests().len(), 4);
}

#[test]
fn create_requires_an_api_base() {
    let sandbox = sandbox(&[], environment(&[]));
    let api_key = SecretString::from("key-1");
    let mut request = create_request("http://sandbox.example", &api_key);
    request.options.api_base = None;
    let error = block_on(sandbox.acreate_sandbox(request, None)).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::MissingApiBase { variable })
            if variable == "OPEN_SANDBOX_API_BASE"
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn create_reads_the_api_base_and_key_from_the_environment() {
    let sandbox = sandbox(
        &[(200, RUNNING), (200, ENDPOINT)],
        environment(&[
            ("OPEN_SANDBOX_API_BASE", "http://env.example"),
            ("OPEN_SANDBOX_API_KEY", "env-key"),
        ]),
    );
    let unused = SecretString::from("unused");
    let mut request = create_request("http://sandbox.example", &unused);
    request.options.api_base = None;
    request.options.api_key = None;
    let handle = block_on(sandbox.acreate_sandbox(request, None)).unwrap();

    assert_eq!(
        handle.container.hidden_params.api_base.as_deref(),
        Some("http://env.example")
    );
    assert_eq!(
        handle
            .container
            .hidden_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("env-key".to_owned())
    );
    assert_eq!(
        sandbox.http.request(0).url.as_str(),
        "http://env.example/sandboxes"
    );
}

#[test]
fn create_omits_the_api_key_header_when_no_key_is_set() {
    let sandbox = sandbox(&[(200, RUNNING), (200, ENDPOINT)], environment(&[]));
    let unused = SecretString::from("unused");
    let mut request = create_request("http://sandbox.example", &unused);
    request.options.api_key = None;
    let handle = block_on(sandbox.acreate_sandbox(request, None)).unwrap();

    assert_eq!(
        header(&sandbox.http.request(0), "open-sandbox-api-key"),
        None
    );
    assert_eq!(
        handle
            .container
            .hidden_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::new())
    );
}

#[test]
fn create_rejects_a_response_without_a_sandbox_id() {
    let sandbox = sandbox(
        &[(200, r#"{"status":{"state":"Running"}}"#)],
        environment(&[]),
    );
    let api_key = SecretString::from("key-1");
    let error =
        block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::Decode { .. })
    ));
    assert_eq!(sandbox.http.requests().len(), 1);
}

#[test]
fn create_reports_a_non_success_status() {
    let sandbox = sandbox(
        &[(401, r#"{"code":401,"message":"invalid key"}"#)],
        environment(&[]),
    );
    let api_key = SecretString::from("key-1");
    let error =
        block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::HttpStatus { status }) if status == 401
    ));
}

#[test]
fn create_sends_every_body_field() {
    let sandbox = sandbox(&[(200, RUNNING), (200, ENDPOINT)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let metadata = BTreeMap::from([("team".to_owned(), "sandbox".to_owned())]);
    let env_vars = BTreeMap::from([("A".to_owned(), "B".to_owned())]);
    let resource_limits = BTreeMap::from([("cpu".to_owned(), "2".to_owned())]);
    let resource_requests = BTreeMap::from([("memory".to_owned(), "1Gi".to_owned())]);
    let entrypoint = vec!["/bin/sh".to_owned()];
    let policy: Box<RawValue> = serde_json::from_str(r#"{"defaultAction":"allow"}"#).unwrap();
    let mut request = create_request("http://sandbox.example", &api_key);
    request.template = Some("custom/image:1");
    request.timeout = Some(42);
    request.allow_internet_access = Some(true);
    request.metadata = Some(&metadata);
    request.env_vars = Some(&env_vars);
    request.resource_limits = Some(&resource_limits);
    request.resource_requests = Some(&resource_requests);
    request.entrypoint = Some(&entrypoint);
    request.network_policy = Some(&policy);
    request.secure_access = true;
    block_on(sandbox.acreate_sandbox(request, None)).unwrap();

    let body = String::from_utf8(sandbox.http.request(0).body).unwrap();
    assert_eq!(
        body,
        concat!(
            r#"{"image":{"uri":"custom/image:1"},"entrypoint":["/bin/sh"],"timeout":42,"#,
            r#""resourceLimits":{"cpu":"2"},"metadata":{"team":"sandbox"},"env":{"A":"B"},"#,
            r#""resourceRequests":{"memory":"1Gi"},"networkPolicy":{"defaultAction":"allow"},"#,
            r#""secureAccess":true}"#
        )
    );
}

#[test]
fn create_uses_the_supplied_client_instead_of_the_pool() {
    let pool = ScriptedHttp::new(&[]);
    let caller = ScriptedHttp::new(&[(200, RUNNING), (200, ENDPOINT)]);
    let sandbox = OpenSandbox {
        http: pool,
        environment: environment(&[]),
        clock: StepClock::new(),
    };
    let api_key = SecretString::from("key-1");
    block_on(sandbox.acreate_sandbox(
        create_request("http://sandbox.example", &api_key),
        Some(&caller),
    ))
    .unwrap();

    assert!(sandbox.http.requests().is_empty());
    assert_eq!(caller.requests().len(), 2);
}

#[test]
fn create_forwards_the_server_proxy_flag_to_the_endpoint_lookup() {
    let sandbox = sandbox(&[(200, RUNNING), (200, ENDPOINT)], environment(&[]));
    let api_key = SecretString::from("key-1");
    let mut request = create_request("http://sandbox.example", &api_key);
    request.options.use_server_proxy = true;
    let handle = block_on(sandbox.acreate_sandbox(request, None)).unwrap();

    assert!(handle.use_server_proxy);
    assert_eq!(
        sandbox.http.request(1).url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=true"
    );
}

#[test]
fn create_rejects_an_api_key_a_header_cannot_carry() {
    let sandbox = sandbox(&[(200, RUNNING), (200, ENDPOINT)], environment(&[]));
    let api_key = SecretString::from("bad\nkey");
    let error =
        block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::InvalidHeader { .. })
    ));
    assert!(sandbox.http.requests().is_empty());
}

#[test]
fn create_never_panics_on_an_unusable_response_body() {
    let sandbox = sandbox(&[(200, "not json")], environment(&[]));
    let api_key = SecretString::from("key-1");
    let error =
        block_on(sandbox.acreate_sandbox(create_request("http://sandbox.example", &api_key), None))
            .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Sandbox(OpenSandboxError::Decode { .. })
    ));
}
