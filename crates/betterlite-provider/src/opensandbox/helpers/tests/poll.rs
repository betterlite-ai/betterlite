use std::collections::BTreeMap;
use std::error::Error as _;
use std::time::Duration;

use http::header::{CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Method};
use rstest::rstest;
use serde_json::value::RawValue;
use tokio::time::Instant;

use super::super::poll::{Endpoint, GetEndpoint, WaitForEndpoint, WaitUntilRunning, sandbox_state};
use super::super::reply::MAX_RESPONSE_BYTES;
use super::super::{MonotonicClock, OpenSandboxError, PollClock as _};
use super::{FakeHttp, block_on, environment, sandbox};

#[derive(Clone, Copy)]
enum Answer {
    MissingEndpoint,
    Decode,
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

const ENDPOINT_BODY: &str = r#"{"endpoint":"10.0.0.5:44772","headers":{"X-Exec-Token":"abc"}}"#;

fn state(body: &str) -> Option<String> {
    let document: Box<RawValue> = serde_json::from_str(body).unwrap();
    sandbox_state(&document)
}

#[rstest]
#[case::running(r#"{"status":{"state":"Running"}}"#, Some("Running"))]
#[case::pending_with_extra_fields(
    r#"{"id":"sbx_1","status":{"state":"Pending"}}"#,
    Some("Pending")
)]
#[case::not_an_object(r#""Running""#, None)]
#[case::array("[]", None)]
#[case::status_is_not_an_object(r#"{"status":"Running"}"#, None)]
#[case::state_is_null(r#"{"status":{"state":null}}"#, None)]
#[case::status_is_absent(r#"{"id":"sbx_1"}"#, None)]
#[case::empty_object("{}", None)]
fn sandbox_state_reads_only_a_nested_state(#[case] body: &str, #[case] expected: Option<&str>) {
    assert_eq!(state(body).as_deref(), expected);
}

#[test]
fn wait_until_running_returns_after_the_first_running_answer() {
    let sandbox = sandbox(
        FakeHttp::new(&[(200, r#"{"status":{"state":"Running"}}"#)]),
        environment(&[]),
    );
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    block_on(sandbox.wait_until_running(&request, None)).unwrap();
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(
        requests[0].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1"
    );
    assert_eq!(
        requests[0].headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(sandbox.clock.elapsed(), Duration::ZERO);
}

#[test]
fn wait_until_running_polls_until_the_state_becomes_running() {
    let sandbox = sandbox(
        FakeHttp::new(&[
            (200, r#"{"status":{"state":"Pending"}}"#),
            (200, r#"{"status":{"state":"Pending"}}"#),
            (200, r#"{"status":{"state":"Running"}}"#),
        ]),
        environment(&[]),
    );
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    block_on(sandbox.wait_until_running(&request, None)).unwrap();
    assert_eq!(sandbox.http.requests().len(), 3);
    assert_eq!(sandbox.clock.elapsed(), Duration::from_millis(400));
}

#[rstest]
#[case::failed("Failed")]
#[case::stopping("Stopping")]
#[case::terminated("Terminated")]
fn wait_until_running_rejects_a_terminal_state(#[case] terminal: &str) {
    let body = format!(r#"{{"status":{{"state":"{terminal}"}}}}"#);
    let sandbox = sandbox(FakeHttp::new(&[(200, body.as_str())]), environment(&[]));
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_until_running(&request, None)).unwrap_err();
    let OpenSandboxError::UnexpectedState { sandbox_id, state } = error else {
        panic!("the terminal state must be rejected");
    };
    assert_eq!(state, terminal);
    assert_eq!(sandbox_id, "sbx_1");
    assert_eq!(sandbox.http.requests().len(), 1);
    assert_eq!(sandbox.clock.elapsed(), Duration::ZERO);
}

#[test]
fn wait_until_running_times_out_with_the_configured_deadline() {
    let sandbox = sandbox(
        FakeHttp::new(&[(200, r#"{"status":{"state":"Pending"}}"#)]),
        environment(&[]),
    );
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_millis(500),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_until_running(&request, None)).unwrap_err();
    assert!(matches!(
        error,
        OpenSandboxError::NotRunning { timeout, .. }
            if timeout == Duration::from_millis(500)
    ));
    assert_eq!(sandbox.http.requests().len(), 4);
    assert_eq!(sandbox.clock.elapsed(), Duration::from_millis(600));
}

#[test]
fn wait_until_running_reports_a_non_success_status() {
    let sandbox = sandbox(FakeHttp::new(&[(500, "")]), environment(&[]));
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_until_running(&request, None)).unwrap_err();
    assert!(matches!(
        error,
        OpenSandboxError::HttpStatus { status: 500 }
    ));
}

#[test]
fn wait_until_running_rejects_a_body_that_is_not_json() {
    let sandbox = sandbox(FakeHttp::new(&[(200, "not json")]), environment(&[]));
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_until_running(&request, None)).unwrap_err();
    assert!(matches!(error, OpenSandboxError::Decode { .. }));
}

#[rstest]
#[case::under_the_cap(MAX_RESPONSE_BYTES - 1, true)]
#[case::at_the_cap(MAX_RESPONSE_BYTES, true)]
#[case::over_the_cap(MAX_RESPONSE_BYTES + 1, false)]
fn wait_until_running_caps_the_response_body(#[case] size: usize, #[case] decoded: bool) {
    let body = "x".repeat(size);
    let sandbox = sandbox(FakeHttp::new(&[(200, body.as_str())]), environment(&[]));
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_until_running(&request, None)).unwrap_err();
    let capped = matches!(
        error,
        OpenSandboxError::ResponseTooLarge { limit } if limit == MAX_RESPONSE_BYTES
    );
    assert_eq!(capped, !decoded);
}

#[tokio::test]
async fn monotonic_clock_tracks_real_time() {
    let clock = MonotonicClock {
        origin: Instant::now(),
    };
    let started = clock.now();
    clock.sleep(Duration::from_millis(20)).await;
    let threshold = started.saturating_add(Duration::from_millis(10));
    assert!(clock.now() >= threshold);
}

#[test]
fn wait_until_running_uses_a_supplied_client_over_the_injected_one() {
    let supplied = FakeHttp::new(&[(200, r#"{"status":{"state":"Running"}}"#)]);
    let sandbox = sandbox(FakeHttp::new(&[]), environment(&[]));
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    block_on(sandbox.wait_until_running(&request, Some(&supplied))).unwrap();
    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(sandbox.http.requests().len(), 0);
}

#[test]
fn wait_until_running_falls_back_to_the_injected_client() {
    let sandbox = sandbox(
        FakeHttp::new(&[(200, r#"{"status":{"state":"Running"}}"#)]),
        environment(&[]),
    );
    let headers = headers();
    let request = WaitUntilRunning {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    block_on(sandbox.wait_until_running(&request, None)).unwrap();
    assert_eq!(sandbox.http.requests().len(), 1);
}

#[test]
fn wait_for_execd_endpoint_returns_the_first_endpoint() {
    let sandbox = sandbox(FakeHttp::new(&[(200, ENDPOINT_BODY)]), environment(&[]));
    let headers = headers();
    let request = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id: "sbx_1",
            api_base: "http://sandbox.example",
            headers: &headers,
            use_server_proxy: true,
        },
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let endpoint = block_on(sandbox.wait_for_execd_endpoint(request, None)).unwrap();
    assert_eq!(
        endpoint,
        Endpoint {
            url: "10.0.0.5:44772".to_owned(),
            headers: BTreeMap::from([("X-Exec-Token".to_owned(), "abc".to_owned())]),
        }
    );
    let requests = sandbox.http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=true"
    );
    assert_eq!(sandbox.clock.elapsed(), Duration::ZERO);
}

#[test]
fn wait_for_execd_endpoint_retries_a_404() {
    let sandbox = sandbox(
        FakeHttp::new(&[(404, ""), (200, ENDPOINT_BODY)]),
        environment(&[]),
    );
    let headers = headers();
    let request = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id: "sbx_1",
            api_base: "http://sandbox.example",
            headers: &headers,
            use_server_proxy: false,
        },
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let endpoint = block_on(sandbox.wait_for_execd_endpoint(request, None)).unwrap();
    assert_eq!(endpoint.url, "10.0.0.5:44772");
    assert_eq!(sandbox.http.requests().len(), 2);
    assert_eq!(sandbox.clock.elapsed(), Duration::from_millis(200));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"endpoint":null}"#)]
#[case::empty(r#"{"endpoint":""}"#)]
fn wait_for_execd_endpoint_retries_a_missing_endpoint(#[case] body: &str) {
    let sandbox = sandbox(
        FakeHttp::new(&[(200, body), (200, ENDPOINT_BODY)]),
        environment(&[]),
    );
    let headers = headers();
    let request = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id: "sbx_1",
            api_base: "http://sandbox.example",
            headers: &headers,
            use_server_proxy: false,
        },
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    block_on(sandbox.wait_for_execd_endpoint(request, None)).unwrap();
    assert_eq!(sandbox.http.requests().len(), 2);
}

#[test]
fn wait_for_execd_endpoint_does_not_retry_another_status() {
    let sandbox = sandbox(FakeHttp::new(&[(500, "")]), environment(&[]));
    let headers = headers();
    let request = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id: "sbx_1",
            api_base: "http://sandbox.example",
            headers: &headers,
            use_server_proxy: false,
        },
        ready_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_for_execd_endpoint(request, None)).unwrap_err();
    assert!(matches!(
        error,
        OpenSandboxError::HttpStatus { status: 500 }
    ));
    assert_eq!(sandbox.http.requests().len(), 1);
}

#[test]
fn wait_for_execd_endpoint_times_out_and_keeps_the_last_error() {
    let sandbox = sandbox(FakeHttp::new(&[(404, "")]), environment(&[]));
    let headers = headers();
    let request = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id: "sbx_1",
            api_base: "http://sandbox.example",
            headers: &headers,
            use_server_proxy: false,
        },
        ready_timeout: Duration::from_millis(500),
        poll_interval: Duration::from_millis(200),
    };
    let error = block_on(sandbox.wait_for_execd_endpoint(request, None)).unwrap_err();
    assert!(matches!(
        error,
        OpenSandboxError::EndpointNotReady { timeout, .. }
            if timeout == Duration::from_millis(500)
    ));
    let source = error.source().unwrap();
    assert_eq!(
        source.to_string(),
        "the OpenSandbox API answered with status 404"
    );
    assert_eq!(sandbox.clock.elapsed(), Duration::from_millis(600));
}

#[rstest]
#[case::absent("{}", Answer::MissingEndpoint)]
#[case::null(r#"{"endpoint":null}"#, Answer::MissingEndpoint)]
#[case::empty(r#"{"endpoint":""}"#, Answer::MissingEndpoint)]
#[case::number(r#"{"endpoint":5}"#, Answer::MissingEndpoint)]
#[case::boolean(r#"{"endpoint":false}"#, Answer::MissingEndpoint)]
#[case::array(r#"{"endpoint":[]}"#, Answer::MissingEndpoint)]
#[case::object(r#"{"endpoint":{}}"#, Answer::MissingEndpoint)]
#[case::not_json("not json", Answer::Decode)]
#[case::not_an_object("[]", Answer::Decode)]
fn get_execd_endpoint_refuses_an_unusable_answer(#[case] body: &str, #[case] answer: Answer) {
    let sandbox = sandbox(FakeHttp::new(&[(200, body)]), environment(&[]));
    let headers = headers();
    let request = GetEndpoint {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        use_server_proxy: false,
    };
    let error = block_on(sandbox.get_execd_endpoint(request, None)).unwrap_err();
    match answer {
        Answer::MissingEndpoint => assert!(matches!(
            error,
            OpenSandboxError::MissingEndpoint { sandbox_id } if sandbox_id == "sbx_1"
        )),
        Answer::Decode => assert!(matches!(error, OpenSandboxError::Decode { .. })),
    }
}

#[test]
fn get_execd_endpoint_writes_the_proxy_flag() {
    let sandbox = sandbox(FakeHttp::new(&[(200, ENDPOINT_BODY)]), environment(&[]));
    let headers = headers();
    let request = GetEndpoint {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        use_server_proxy: false,
    };
    block_on(sandbox.get_execd_endpoint(request, None)).unwrap();
    assert_eq!(
        sandbox.http.requests()[0].url.as_str(),
        "http://sandbox.example/sandboxes/sbx_1/endpoints/44772?use_server_proxy=false"
    );
}

#[test]
fn get_execd_endpoint_reads_string_headers_only_from_an_object() {
    let sandbox = sandbox(
        FakeHttp::new(&[(
            200,
            r#"{"endpoint":"host:1","headers":{"X-Exec-Token":"abc","n":5}}"#,
        )]),
        environment(&[]),
    );
    let headers = headers();
    let request = GetEndpoint {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        use_server_proxy: false,
    };
    let endpoint = block_on(sandbox.get_execd_endpoint(request, None)).unwrap();
    assert_eq!(
        endpoint.headers,
        BTreeMap::from([
            ("X-Exec-Token".to_owned(), "abc".to_owned()),
            ("n".to_owned(), "5".to_owned()),
        ])
    );
}

#[test]
fn get_execd_endpoint_has_no_headers_without_a_headers_member() {
    let sandbox = sandbox(
        FakeHttp::new(&[(200, r#"{"endpoint":"host:1"}"#)]),
        environment(&[]),
    );
    let headers = headers();
    let request = GetEndpoint {
        sandbox_id: "sbx_1",
        api_base: "http://sandbox.example",
        headers: &headers,
        use_server_proxy: false,
    };
    let endpoint = block_on(sandbox.get_execd_endpoint(request, None)).unwrap();
    assert!(endpoint.headers.is_empty());
}
