#![cfg(test)]

use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::AgentsDeleteVersionsError;
use super::asynchronous::async_delete_agent;
use super::support::{
    Call, Fixture, FixtureError, assert_provider_refusal, caller_headers, clients, dead_address,
    exposed, failing_environment, recorded_error, reply_with_bytes, resolved_timeout,
};
use super::synchronous::delete_agent;
use crate::interactions::base::tests::support as server;
use crate::interactions::base::{ClientError, Logging, RequestFailure};

fn fixture() -> Fixture {
    let mut fixture = Fixture::default();
    fixture
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    fixture
}

#[test]
fn delete_agent_sends_the_delete_to_the_configured_url() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let output = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(output, "delete|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("DELETE /v1beta/agents/agent-1 HTTP/1.1"),
        "{sent}"
    );
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    assert_eq!(
        fixture.calls(),
        vec![
            "environment|0|1".to_owned(),
            format!("delete_request|agent-1|http://{}|1", serving.address),
            "delete_response|agent-1|200".to_owned(),
        ]
    );
}

#[test]
fn delete_agent_takes_the_api_base_from_litellm_params() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text(format!("http://{}/base", serving.address)),
    );

    delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("DELETE /base/v1beta/agents/agent-1 HTTP/1.1"),
        "{sent}"
    );
    let pre = logging.pre_call.as_ref().unwrap();
    assert_eq!(
        pre.api_base,
        format!("http://{}/base/v1beta/agents/agent-1", serving.address)
    );
}

#[test]
fn delete_agent_reports_a_non_string_api_base() {
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(dead_address());
    call.params
        .insert("api_base".to_owned(), JsonValue::Bool(true));

    let error = delete_agent(
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
            AgentsDeleteVersionsError::Parameters(ClientError::FieldType {
                field: "api_base",
                ..
            })
        ),
        "{error:?}"
    );
    assert_eq!(fixture.calls(), vec!["environment|0|1".to_owned()]);
    assert!(logging.pre_call.is_none());
}

#[test]
fn delete_agent_logs_the_pre_call_and_the_post_call() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    delete_agent(
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
        format!("http://{}/v1beta/agents/agent-1", serving.address)
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
fn delete_agent_records_a_reply_that_is_not_text() {
    let serving = server::serve_once(reply_with_bytes("200 OK", b"deleted \xff"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let output = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(output, "delete|agent-1|200|9");
    assert_eq!(
        logging.model_call_details.get("original_response"),
        Some(&JsonValue::Text("deleted ".to_owned()))
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_agent_uses_the_caller_timeout_when_given() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(300)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(30));

    let error = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentsDeleteVersionsError::Transport(RequestFailure::Transport(source))
                if source.is_timeout()
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_agent_lets_the_caller_timeout_win_over_the_request_timeout() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(300)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(30));
    call.request_timeout = resolved_timeout(&[("REQUEST_TIMEOUT", "30")]);

    let error = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentsDeleteVersionsError::Transport(RequestFailure::Transport(source))
                if source.is_timeout()
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_agent_sends_the_request_timeout_the_environment_names() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(400)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.request_timeout = resolved_timeout(&[("REQUEST_TIMEOUT", "0.05")]);

    let error = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentsDeleteVersionsError::Transport(RequestFailure::Transport(source))
                if source.is_timeout()
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_agent_reports_an_unusable_request_timeout() {
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(dead_address());
    call.request_timeout = RequestTimeout {
        seconds: f64::NAN,
        explicitly_set: true,
    };

    let error = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(error, AgentsDeleteVersionsError::Timeout { seconds, .. } if seconds.is_nan()),
        "{error:?}"
    );
    assert!(logging.pre_call.is_some());
    assert!(logging.model_call_details.is_empty());
}

#[test]
fn delete_agent_returns_the_provider_error_for_a_failing_status() {
    let serving = server::serve_once(
        server::reply("404 Not Found", &[("retry-after", "7")], "missing"),
        None,
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let error = delete_agent(
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
            .any(|record| record.starts_with("delete_response")),
        "{calls:?}"
    );
    assert!(logging.model_call_details.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn delete_agent_passes_a_connection_failure_through() {
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = delete_agent(
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
fn delete_agent_reports_a_failing_environment_without_logging() {
    let fixture = failing_environment();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = delete_agent(
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
    assert!(logging.model_call_details.is_empty());
}

#[rstest]
#[case::absent(&[], "environment|0|1")]
#[case::given(&[("x-caller", "c1")], "environment|1|1")]
fn delete_agent_sends_the_caller_headers_to_validate_environment(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.extra_headers = caller_headers(extra);

    delete_agent(
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
async fn async_delete_agent_sends_the_delete_to_the_configured_url() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let output = async_delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "delete|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("DELETE /v1beta/agents/agent-1 HTTP/1.1"),
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
async fn async_delete_agent_sends_the_caller_timeout() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(300)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(30));

    let error = async_delete_agent(
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
            &error,
            AgentsDeleteVersionsError::Transport(RequestFailure::Transport(source))
                if source.is_timeout()
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_agent_uses_the_request_timeout_when_the_caller_gives_none() {
    let serving = server::serve_once(
        server::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(400)),
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.request_timeout = resolved_timeout(&[("REQUEST_TIMEOUT", "0.05")]);

    let error = async_delete_agent(
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
            &error,
            AgentsDeleteVersionsError::Transport(RequestFailure::Transport(source))
                if source.is_timeout()
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_agent_returns_the_provider_error_for_a_failing_status() {
    let serving = server::serve_once(
        server::reply("500 Internal Server Error", &[], "boom"),
        None,
    );
    let fixture = fixture();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(serving.address);

    let error = async_delete_agent(
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
            .any(|record| record.starts_with("delete_response"))
    );
    assert!(logging.model_call_details.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_agent_reports_a_failing_environment_without_logging() {
    let fixture = failing_environment();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = async_delete_agent(
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
