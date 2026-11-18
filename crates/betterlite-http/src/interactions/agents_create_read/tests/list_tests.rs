#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;

use super::AgentRequestError;
use super::list_async::async_list_agents;
use super::list_sync::list_agents;
use super::support::{self, Config, FixtureError, Recorder, caller_headers, clients, params_at};
use crate::interactions::base::tests::support as base;
use betterlite_http_fixture::server::serve;

#[test]
fn list_agents_gets_the_configured_url_and_query() {
    let serving = serve(
        base::reply("200 OK", &[], r#"{"agents":[]}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut config = Config::for_address(serving.address);
    config.list.params = base::params(r#"{"pageSize": 2, "pageToken": "t1"}"#);
    config
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let params = params_at(serving.address);
    let mut request = support::list_request(&params);
    request.extra_headers = caller_headers(&[("x-caller", "c1")]);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = list_agents(&config, &mut clients, &mut logging, None, &request).unwrap();

    assert_eq!(output, r#"|200|{"agents":[]}"#);
    let address = serving.address;
    let sent = serving.finish().unwrap();
    assert!(
        sent.head
            .starts_with("GET /v1beta/agents?pageSize=2&pageToken=t1 HTTP/1.1"),
        "{}",
        sent.head
    );
    assert!(
        sent.head.contains("x-goog-api-key: key-1\r\n"),
        "{}",
        sent.head
    );
    assert_eq!(
        config.calls(),
        vec![
            "environment|1|1".to_owned(),
            format!("list_request|http://{address}"),
        ]
    );
}

#[test]
fn list_agents_logs_the_list_input_and_empty_additional_arguments() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let mut config = Config::for_address(serving.address);
    config
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
    )
    .unwrap();

    let url = format!("http://{}/v1beta/agents", serving.address);
    assert_eq!(
        logging.calls(),
        vec![
            format!("pre|list_agents||{url}|x-goog-api-key=key-1|-"),
            "post|{}|-".to_owned(),
        ]
    );
    serving.finish().unwrap();
}

#[test]
fn list_agents_accepts_and_ignores_the_timeout() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(100)).unwrap();
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::list_request(&params);
    request.timeout = Some(Duration::from_millis(1));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = list_agents(&config, &mut clients, &mut logging, None, &request).unwrap();

    assert_eq!(output, "|200|{}");
    serving.finish().unwrap();
}

#[test]
fn list_agents_passes_a_failing_status_to_the_response_transform() {
    let serving = serve(base::reply("404 Not Found", &[], "missing"), Duration::ZERO).unwrap();
    let mut config = Config::for_address(serving.address);
    config.refuse_status = Some(404);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
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
    serving.finish().unwrap();
}

#[rstest]
#[case::absent(&[], "environment|0|1")]
#[case::given(&[("x-caller", "c1")], "environment|1|1")]
fn list_agents_sends_the_caller_headers_to_validate_environment(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::list_request(&params);
    request.extra_headers = caller_headers(extra);
    let mut clients = clients();
    let mut logging = Recorder::default();

    list_agents(&config, &mut clients, &mut logging, None, &request).unwrap();

    assert_eq!(config.calls().first().map(String::as_str), Some(expected));
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_list_agents_gets_the_configured_url_and_query() {
    let serving = serve(
        base::reply("200 OK", &[], r#"{"agents":[]}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut config = Config::for_address(serving.address);
    config.list.params = base::params(r#"{"pageSize": 2, "pageToken": "t1"}"#);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = async_list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
    )
    .await
    .unwrap();

    assert_eq!(output, r#"|200|{"agents":[]}"#);
    let sent = serving.finish().unwrap();
    assert!(
        sent.head
            .starts_with("GET /v1beta/agents?pageSize=2&pageToken=t1 HTTP/1.1"),
        "{}",
        sent.head
    );
}

#[tokio::test]
async fn async_list_agents_logs_the_list_input_and_empty_additional_arguments() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let mut config = Config::for_address(serving.address);
    config
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
    )
    .await
    .unwrap();

    let url = format!("http://{}/v1beta/agents", serving.address);
    assert_eq!(
        logging.calls(),
        vec![
            format!("pre|list_agents||{url}|x-goog-api-key=key-1|-"),
            "post|{}|-".to_owned(),
        ]
    );
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_list_agents_accepts_and_ignores_the_timeout() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(100)).unwrap();
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut request = support::list_request(&params);
    request.timeout = Some(Duration::from_millis(1));
    let mut clients = clients();
    let mut logging = Recorder::default();

    let output = async_list_agents(&config, &mut clients, &mut logging, None, &request)
        .await
        .unwrap();

    assert_eq!(output, "|200|{}");
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_list_agents_passes_a_failing_status_to_the_response_transform() {
    let serving = serve(
        base::reply("500 Internal Server Error", &[], "boom"),
        Duration::ZERO,
    )
    .unwrap();
    let mut config = Config::for_address(serving.address);
    config.refuse_status = Some(500);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
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
    assert!(
        config
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    serving.finish().unwrap();
}
