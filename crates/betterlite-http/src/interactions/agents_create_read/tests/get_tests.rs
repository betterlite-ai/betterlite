#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;

use super::AgentRequestError;
use super::get_async::async_get_agent;
use super::get_sync::get_agent;
use super::support::{self, Config, FixtureError, Recorder, caller_headers, clients, params_at};
use crate::interactions::base::tests::support as base;

#[test]
fn get_agent_gets_the_configured_url_and_query() {
    let serving = base::serve_once(base::reply("200 OK", &[], r#"{"id":"agent-1"}"#), None);
    let mut config = Config::for_address(serving.address);
    config.item.params = base::params(r#"{"alt": "json"}"#);
    config
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .unwrap();

    assert_eq!(output, r#"agent-1|200|{"id":"agent-1"}"#);
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/agents/agent-1?alt=json HTTP/1.1"),
        "{sent}"
    );
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    assert_eq!(
        config.calls(),
        vec![
            "environment|0|1".to_owned(),
            format!("get_request|agent-1|http://{}", serving.address),
        ]
    );
}

#[test]
fn get_agent_passes_the_name_to_both_transforms_and_logs_it() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .unwrap();

    assert_eq!(output, "agent-1|200|{}");
    let calls = config.calls();
    assert!(
        calls.contains(&format!("get_request|agent-1|http://{}", serving.address)),
        "{calls:?}"
    );
    let url = format!("http://{}/v1beta/agents/agent-1", serving.address);
    assert_eq!(
        logging.calls(),
        vec![format!("pre|agent-1||{url}||-"), "post|{}|-".to_owned()]
    );
    serving.request.join().unwrap();
}

#[test]
fn get_agent_accepts_and_ignores_the_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(100)),
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::get_request("agent-1", &params);
    request.timeout = Some(Duration::from_millis(1));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = get_agent(&config, &mut clients, &mut logging, None, &request).unwrap();

    assert_eq!(output, "agent-1|200|{}");
    serving.request.join().unwrap();
}

#[test]
fn get_agent_passes_a_failing_status_to_the_response_transform() {
    let serving = base::serve_once(base::reply("404 Not Found", &[], "missing"), None);
    let mut config = Config::for_address(serving.address);
    config.refuse_status = Some(404);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Refused(404))
        ),
        "{error:?}"
    );
    assert!(
        config
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    let post = logging
        .calls()
        .into_iter()
        .find(|call| call.starts_with("post|"));
    assert_eq!(post.as_deref(), Some("post|missing|-"));
    serving.request.join().unwrap();
}

#[rstest]
#[case::absent(&[], "environment|0|1")]
#[case::given(&[("x-caller", "c1")], "environment|1|1")]
fn get_agent_sends_the_caller_headers_to_validate_environment(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::get_request("agent-1", &params);
    request.extra_headers = caller_headers(extra);
    let mut clients = clients();
    let mut logging = Recorder::default();

    get_agent(&config, &mut clients, &mut logging, None, &request).unwrap();

    assert_eq!(config.calls().first().map(String::as_str), Some(expected));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_agent_gets_the_configured_url_and_query() {
    let serving = base::serve_once(base::reply("200 OK", &[], r#"{"id":"agent-1"}"#), None);
    let mut config = Config::for_address(serving.address);
    config.item.params = base::params(r#"{"alt": "json"}"#);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = async_get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .await
    .unwrap();

    assert_eq!(output, r#"agent-1|200|{"id":"agent-1"}"#);
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/agents/agent-1?alt=json HTTP/1.1"),
        "{sent}"
    );
}

#[tokio::test]
async fn async_get_agent_passes_the_name_to_both_transforms_and_logs_it() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = async_get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .await
    .unwrap();

    assert_eq!(output, "agent-1|200|{}");
    let url = format!("http://{}/v1beta/agents/agent-1", serving.address);
    assert_eq!(
        logging.calls(),
        vec![format!("pre|agent-1||{url}||-"), "post|{}|-".to_owned()]
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_agent_accepts_and_ignores_the_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(100)),
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::get_request("agent-1", &params);
    request.timeout = Some(Duration::from_millis(1));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = async_get_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap();

    assert_eq!(output, "agent-1|200|{}");
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_agent_passes_a_failing_status_to_the_response_transform() {
    let serving = base::serve_once(base::reply("500 Internal Server Error", &[], "boom"), None);
    let mut config = Config::for_address(serving.address);
    config.refuse_status = Some(500);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Refused(500))
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}
