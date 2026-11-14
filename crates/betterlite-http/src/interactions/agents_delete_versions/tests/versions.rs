#![cfg(test)]

use std::time::Duration;

use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::AgentsDeleteVersionsError;
use super::asynchronous::async_list_agent_versions;
use super::support::{
    Call, Fixture, FixtureError, assert_provider_refusal, caller_headers, clients, dead_address,
    exposed, failing_environment, recorded_error, resolved_timeout,
};
use super::synchronous::list_agent_versions;
use crate::interactions::base::tests::support as server;
use crate::interactions::base::{Logging, RequestFailure};

fn fixture() -> Fixture {
    let mut fixture = Fixture::default();
    fixture
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    fixture.query = server::params(r#"{"pageSize": 5, "pageToken": "t1"}"#);
    fixture
}

#[test]
fn versions_sends_the_get_with_the_transformed_query() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let output = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(output, "versions|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/agents/agent-1/versions?pageSize=5&pageToken=t1 HTTP/1.1"),
        "{sent}"
    );
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    assert_eq!(
        fixture.calls(),
        vec![
            "environment|0|1".to_owned(),
            format!("versions_request|agent-1|http://{}|1", serving.address),
            "versions_response|agent-1|200".to_owned(),
        ]
    );
}

#[test]
fn versions_takes_the_api_base_from_litellm_params() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text(format!("http://{}/base", serving.address)),
    );

    list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with(
            "GET /base/v1beta/agents/agent-1/versions?pageSize=5&pageToken=t1 HTTP/1.1"
        ),
        "{sent}"
    );
}

#[test]
fn versions_logs_the_pre_call_and_the_post_call() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    let pre = logging.pre_call.as_ref().unwrap();
    assert_eq!(pre.input, "agent-1");
    assert_eq!(pre.api_key.expose_secret(), "");
    assert_eq!(
        pre.api_base,
        format!("http://{}/v1beta/agents/agent-1/versions", serving.address)
    );
    assert_eq!(exposed(&pre.headers, "x-goog-api-key"), "key-1");
    assert_eq!(logging.model_call_details.len(), 2);
    assert_eq!(
        logging.model_call_details.get("original_response"),
        Some(&JsonValue::Text("{}".to_owned()))
    );
    assert_eq!(
        logging.model_call_details.get("additional_args"),
        Some(&JsonValue::Object(IndexMap::new()))
    );
    serving.request.join().unwrap();
}

#[test]
fn versions_sends_no_timeout() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(100)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(1));
    call.request_timeout = resolved_timeout(&[("REQUEST_TIMEOUT", "0.01")]);

    let output = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(output, "versions|agent-1|200|2");
    serving.request.join().unwrap();
}

#[test]
fn versions_returns_the_provider_error_for_a_failing_status() {
    let serving = server::serve_once(
        server::reply("404 Not Found", &[("retry-after", "7")], "missing"),
        None,
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let error = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert_provider_refusal(&error, 404);
    let calls = fixture.calls();
    assert_eq!(
        recorded_error(&calls),
        Some("error|404|missing|7"),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|record| record.starts_with("versions_response")),
        "{calls:?}"
    );
    assert!(logging.pre_call.is_some());
    assert!(logging.model_call_details.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn versions_passes_a_connection_failure_through() {
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            AgentsDeleteVersionsError::Transport(RequestFailure::Transport(_))
        ),
        "{error:?}"
    );
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|record| record.starts_with("error|"))
    );
}

#[test]
fn versions_reports_a_failing_environment_without_logging() {
    let mut fixture = fixture();
    fixture.fail_environment = true;
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            AgentsDeleteVersionsError::Provider(FixtureError::Field("headers"))
        ),
        "{error:?}"
    );
    assert_eq!(fixture.calls(), vec!["environment|0|1".to_owned()]);
    assert_eq!(clients.sync_clients.len(), 1);
    assert!(logging.pre_call.is_none());
}

#[rstest]
#[case::absent(&[], "environment|0|1")]
#[case::given(&[("x-caller", "c1")], "environment|1|1")]
fn versions_sends_the_caller_headers_to_validate_environment(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.extra_headers = caller_headers(extra);

    list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(fixture.calls().first().map(String::as_str), Some(expected));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_versions_sends_the_get_with_the_transformed_query() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let output = async_list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "versions|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/agents/agent-1/versions?pageSize=5&pageToken=t1 HTTP/1.1"),
        "{sent}"
    );
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    let pre = logging.pre_call.as_ref().unwrap();
    assert_eq!(pre.input, "agent-1");
    assert_eq!(
        logging.model_call_details.get("original_response"),
        Some(&JsonValue::Text("{}".to_owned()))
    );
}

#[tokio::test]
async fn async_versions_sends_no_timeout() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(100)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(1));
    call.request_timeout = resolved_timeout(&[("REQUEST_TIMEOUT", "0.01")]);

    let output = async_list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "versions|agent-1|200|2");
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_versions_returns_the_provider_error_for_a_failing_status() {
    let serving = server::serve_once(
        server::reply("500 Internal Server Error", &[], "boom"),
        None,
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let error = async_list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap_err();

    assert_provider_refusal(&error, 500);
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|record| record.starts_with("versions_response"))
    );
    assert!(logging.model_call_details.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_versions_reports_a_failing_environment_without_logging() {
    let fixture = failing_environment();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = async_list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            error,
            AgentsDeleteVersionsError::Provider(FixtureError::Field("headers"))
        ),
        "{error:?}"
    );
    assert_eq!(clients.async_clients.len(), 1);
    assert!(logging.pre_call.is_none());
    assert!(logging.model_call_details.is_empty());
}
