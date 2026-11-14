#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;

use super::AgentRequestError;
use super::create_async::async_create_agent;
use super::create_sync::create_agent;
use super::support::{self, Config, FixtureError, Recorder, caller_headers, clients, params_at};
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{ClientError, RequestFailure};

#[test]
fn create_agent_posts_the_configured_request_and_returns_the_transform() {
    let serving = support::serve_post(base::reply("200 OK", &[], r#"{"id":"agent-1"}"#), None);
    let mut config = Config::for_address(serving.address);
    config.create_body = base::params(r#"{"name": "agent-1", "instructions": "hi"}"#);
    config.headers.insert(
        "content-type".to_owned(),
        SecretString::from("application/json"),
    );
    config
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.extra_headers = caller_headers(&[("x-caller", "c1")]);
    request.timeout = Some(Duration::from_secs(1));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap();

    assert_eq!(output, r#"agent-1|200|{"id":"agent-1"}"#);
    let sent = serving.request.join().unwrap();
    assert!(sent.starts_with("POST /v1beta/agents HTTP/1.1"), "{sent}");
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    assert!(
        sent.contains("content-type: application/json\r\n"),
        "{sent}"
    );
    assert_eq!(
        support::sent_body(&sent),
        base::params(r#"{"name": "agent-1", "instructions": "hi"}"#)
    );
    assert_eq!(
        config.calls(),
        vec![
            "environment|1|1".to_owned(),
            format!("create_url|http://{}", serving.address),
            "create_request|agent-1".to_owned(),
        ]
    );
}

#[test]
fn create_agent_logs_the_call_and_the_additional_arguments() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.create_body = base::params(r#"{"name": "agent-1"}"#);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.extra_headers = caller_headers(&[("x-caller", "c1")]);
    let mut clients = clients();
    let mut logging = Recorder::default();

    create_agent(&config, &mut clients, &mut logging, None, &request).unwrap();

    let url = format!("http://{}/v1beta/agents", serving.address);
    assert_eq!(
        logging.calls(),
        vec![
            format!(r#"pre|agent-1||{url}||{{"name":"agent-1"}}"#),
            r#"post|{}|{"name":"agent-1"}"#.to_owned(),
        ]
    );
    serving.request.join().unwrap();
}

#[test]
fn create_agent_lets_extra_body_replace_the_transformed_body() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.create_body = base::params(r#"{"name": "agent-1", "temperature": 0.5}"#);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.extra_body = Some(base::params(r#"{"name": "agent-two", "new": true}"#));
    let mut clients = clients();
    let mut logging = Recorder::default();

    create_agent(&config, &mut clients, &mut logging, None, &request).unwrap();

    let sent = serving.request.join().unwrap();
    assert_eq!(
        support::sent_body(&sent),
        base::params(r#"{"name": "agent-two", "temperature": 0.5, "new": true}"#)
    );
    let post = &logging.calls()[1];
    assert!(post.contains(r#""name":"agent-two""#), "{post}");
}

#[test]
fn create_agent_sends_the_callers_timeout() {
    let serving = support::serve_post(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.timeout = Some(Duration::from_millis(20));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap_err();

    assert!(
        matches!(&error, AgentRequestError::Transport(RequestFailure::Transport(source)) if source.is_timeout()),
        "{error:?}"
    );
    assert!(
        logging
            .calls()
            .iter()
            .all(|call| !call.starts_with("post|"))
    );
    serving.request.join().unwrap();
}

#[test]
fn create_agent_falls_back_to_the_request_timeout() {
    let serving = support::serve_post(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.request_timeout = Duration::from_millis(20);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap_err();

    assert!(
        matches!(&error, AgentRequestError::Transport(RequestFailure::Transport(source)) if source.is_timeout()),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[test]
fn create_agent_converts_a_failing_status_through_the_config() {
    let serving = support::serve_post(
        base::reply(
            "429 Too Many Requests",
            &[("retry-after", "1")],
            "slow down",
        ),
        None,
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Refused(429))
        ),
        "{error:?}"
    );
    assert_eq!(
        config.calls(),
        vec![
            "environment|0|1".to_owned(),
            format!("create_url|http://{}", serving.address),
            "create_request|agent-1".to_owned(),
            "error|429|slow down|3".to_owned(),
        ]
    );
    assert!(
        logging
            .calls()
            .iter()
            .all(|call| !call.starts_with("post|"))
    );
    serving.request.join().unwrap();
}

#[test]
fn create_agent_reports_a_redirect_status_as_an_error() {
    let serving = support::serve_post(
        base::reply(
            "302 Found",
            &[("location", "http://elsewhere.invalid/agents")],
            "",
        ),
        None,
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Refused(302))
        ),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[test]
fn create_agent_builds_the_client_before_a_failing_environment() {
    let mut config = Config::for_address(support::dead_address());
    config.refuse_environment = true;
    let params = params_at(support::dead_address());
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Environment)
        ),
        "{error:?}"
    );
    assert_eq!(config.calls(), vec!["environment|0|1".to_owned()]);
    assert!(logging.calls().is_empty());
    assert_eq!(clients.sync_clients.len(), 1);
}

#[test]
fn create_agent_reports_a_non_string_api_base() {
    let config = Config::for_address(support::dead_address());
    let params = base::params(r#"{"api_base": 7}"#);
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(&config, &mut clients, &mut logging, None, &request).unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Parameters(ClientError::FieldType {
                field: "api_base",
                ..
            })
        ),
        "{error:?}"
    );
    assert!(config.calls().is_empty());
    assert_eq!(clients.sync_clients.len(), 1);
}

#[test]
fn body_text_reads_non_utf8_bytes_as_latin1() {
    assert_eq!(super::body_text(&[b'a', 0xff]), "a\u{ff}");
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"api_base": null}"#)]
fn create_agent_passes_an_empty_api_base_to_the_url_transform(#[case] params_text: &str) {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = base::params(params_text);
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    create_agent(&config, &mut clients, &mut logging, None, &request).unwrap();

    let calls = config.calls();
    assert!(
        calls.iter().any(|call| call.as_str() == "create_url|"),
        "{calls:?}"
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_agent_posts_the_configured_request_and_returns_the_transform() {
    let serving = support::serve_post(base::reply("200 OK", &[], r#"{"id":"agent-1"}"#), None);
    let mut config = Config::for_address(serving.address);
    config.create_body = base::params(r#"{"name": "agent-1"}"#);
    config
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let params = params_at(serving.address);
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = async_create_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap();

    assert_eq!(output, r#"agent-1|200|{"id":"agent-1"}"#);
    let sent = serving.request.join().unwrap();
    assert!(sent.starts_with("POST /v1beta/agents HTTP/1.1"), "{sent}");
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    assert_eq!(
        support::sent_body(&sent),
        base::params(r#"{"name": "agent-1"}"#)
    );
}

#[tokio::test]
async fn async_create_agent_lets_extra_body_replace_the_transformed_body() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.create_body = base::params(r#"{"name": "agent-1"}"#);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.extra_body = Some(base::params(r#"{"name": "agent-two"}"#));
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_create_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap();

    let sent = serving.request.join().unwrap();
    assert_eq!(
        support::sent_body(&sent),
        base::params(r#"{"name": "agent-two"}"#)
    );
}

#[tokio::test]
async fn async_create_agent_sends_the_callers_timeout() {
    let serving = support::serve_post(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.timeout = Some(Duration::from_millis(20));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_create_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AgentRequestError::Transport(RequestFailure::Transport(source)) if source.is_timeout()),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_agent_falls_back_to_the_request_timeout() {
    let serving = support::serve_post(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::create_request("agent-1", &params);
    request.request_timeout = Duration::from_millis(20);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_create_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AgentRequestError::Transport(RequestFailure::Transport(source)) if source.is_timeout()),
        "{error:?}"
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_agent_converts_a_failing_status_through_the_config() {
    let serving = support::serve_post(base::reply("500 Internal Server Error", &[], "boom"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_create_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Refused(500))
        ),
        "{error:?}"
    );
    assert!(
        logging
            .calls()
            .iter()
            .all(|call| !call.starts_with("post|"))
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_agent_builds_the_client_before_a_failing_environment() {
    let mut config = Config::for_address(support::dead_address());
    config.refuse_environment = true;
    let params = params_at(support::dead_address());
    let request = support::create_request("agent-1", &params);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_create_agent(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Provider(FixtureError::Environment)
        ),
        "{error:?}"
    );
    assert_eq!(config.calls(), vec!["environment|0|1".to_owned()]);
    assert!(logging.calls().is_empty());
    assert_eq!(clients.async_clients.len(), 1);
}
