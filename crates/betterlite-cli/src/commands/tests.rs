#![cfg(test)]

pub(crate) mod support;

use std::net::TcpListener;
use std::time::{Duration, Instant};

use betterlite_http::error::HttpError;
use http::Method;
use rstest::rstest;

use crate::commands::tables::print_http_error;
use crate::commands::{BODY_LIMIT_BYTES, Client, ClientError, Payload, REQUEST_TIMEOUT, Request};
use crate::pkce_login::{Http as _, HttpBody, models::PkceFailure};

use support::{Answer, TestServer, client, client_for, client_with_environment, secret};

fn get(client: &Client, path: &str) -> Request {
    Request {
        method: Method::GET,
        url: client.resolve(path).unwrap(),
        headers: Vec::new(),
        payload: Payload::Empty,
    }
}

#[test]
fn client_strips_every_trailing_slash_from_the_base_url() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let client = client(&format!("{}///", server.base_url), None);

    let response = client.send(get(&client, "/credentials")).unwrap();

    assert_eq!(response.status, 200);
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /credentials HTTP/1.1");
}

#[rstest]
#[case::with_a_key(Some("sk-test"), Some("Bearer sk-test"))]
#[case::without_a_key(None, None)]
fn client_adds_the_bearer_header_only_when_a_key_is_given(
    #[case] api_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let client = client_for(&server, api_key);

    client.send(get(&client, "/credentials")).unwrap();

    let requests = server.finish();
    assert_eq!(requests[0].header("authorization"), expected);
}

#[test]
fn client_sends_a_json_body_with_its_content_type() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let client = client_for(&server, Some("sk-test"));
    let url = format!("{}/token", server.base_url);

    let response = client
        .post(
            &url,
            HttpBody::Json(br#"{"grant_type":"authorization_code"}"#.to_vec()),
            15,
            false,
        )
        .unwrap();

    assert_eq!(response.status, 200);
    let requests = server.finish();
    assert_eq!(requests[0].method(), "POST");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(
        requests[0].body_text(),
        "{\"grant_type\":\"authorization_code\"}"
    );
    assert_eq!(requests[0].header("authorization"), None);
}

#[test]
fn client_sends_a_form_body_url_encoded() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let client = client_for(&server, None);
    let url = format!("{}/token", server.base_url);

    client
        .post(
            &url,
            HttpBody::Form(vec![
                (String::from("grant_type"), secret("authorization_code")),
                (String::from("code"), secret("a b")),
            ]),
            15,
            false,
        )
        .unwrap();

    let requests = server.finish();
    assert_eq!(
        requests[0].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        requests[0].body_text(),
        "grant_type=authorization_code&code=a+b"
    );
}

#[test]
fn client_answers_a_redirect_as_the_response_instead_of_following_it() {
    let target = TestServer::start(Vec::new());
    let location = format!("{}/secret", target.base_url);
    let origin = TestServer::start(vec![
        Answer::empty(302, "Found").with_header("location", &location),
    ]);
    let client = client_for(&origin, Some("sk-test"));
    let url = format!("{}/token", origin.base_url);

    let response = client
        .post(
            &url,
            HttpBody::Form(vec![(String::from("code"), secret("abc"))]),
            15,
            false,
        )
        .unwrap();

    assert_eq!(response.status, 302);
    let found = response
        .headers
        .iter()
        .find(|(name, _value)| name == "location")
        .map(|(_name, value)| value.as_str());
    assert_eq!(found, Some(location.as_str()));
    assert!(target.finish().is_empty());
}

#[test]
fn client_maps_a_transport_failure_to_a_pkce_transport_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = client(&format!("http://{address}"), None);

    let error = client
        .get(
            &format!("http://{address}/.well-known/litellm-cli-auth"),
            15,
        )
        .unwrap_err();

    assert!(matches!(error, PkceFailure::Transport { .. }), "{error:?}");
}

#[test]
fn client_rejects_a_body_declared_over_the_sixty_four_mebibyte_cap() {
    let server = TestServer::start(vec![Answer::json(200, "{}").declared(BODY_LIMIT_BYTES + 1)]);
    let client = client_for(&server, None);

    let error = client.send(get(&client, "/credentials")).unwrap_err();

    assert!(
        matches!(error, ClientError::Transport(HttpError::TooLarge { limit }) if limit == BODY_LIMIT_BYTES),
        "{error:?}"
    );
}

#[test]
fn client_reads_a_body_declared_exactly_at_the_cap() {
    let server = TestServer::start(vec![Answer::json(200, "{}").declared(BODY_LIMIT_BYTES)]);
    let client = client_for(&server, None);

    let error = client.send(get(&client, "/credentials")).unwrap_err();

    assert!(
        matches!(error, ClientError::Transport(HttpError::Request(_))),
        "{error:?}"
    );
}

#[test]
fn client_bounds_a_request_by_the_timeout_it_is_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}").paused(Duration::from_secs(2))]);
    let client = client_for(&server, None);
    let started = Instant::now();

    let error = client
        .get(&format!("{}/credentials", server.base_url), 1)
        .unwrap_err();

    assert!(
        matches!(
            error,
            PkceFailure::Transport {
                source: HttpError::Timeout
            }
        ),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
}

#[test]
fn client_uses_a_thirty_second_request_timeout() {
    assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(30));
}

#[test]
fn client_routes_a_request_through_the_proxy_in_the_environment() {
    let proxy = TestServer::start(vec![Answer::json(200, "{}")]);
    let client = client_with_environment(
        "http://proxy-target.invalid",
        None,
        &[("http_proxy", proxy.base_url.as_str())],
    );

    let response = client.send(get(&client, "/credentials")).unwrap();

    assert_eq!(response.status, 200);
    let requests = proxy.finish();
    assert_eq!(
        requests[0].request_line,
        "GET http://proxy-target.invalid/credentials HTTP/1.1"
    );
}

#[test]
fn client_bypasses_the_proxy_for_a_host_in_no_proxy() {
    let proxy = TestServer::start(Vec::new());
    let origin = TestServer::start(vec![Answer::json(200, "{}")]);
    let client = client_with_environment(
        &origin.base_url,
        None,
        &[
            ("http_proxy", proxy.base_url.as_str()),
            ("no_proxy", "127.0.0.1"),
        ],
    );

    let response = client.send(get(&client, "/credentials")).unwrap();

    assert_eq!(response.status, 200);
    let requests = origin.finish();
    assert_eq!(requests[0].path(), "/credentials");
    assert!(proxy.finish().is_empty());
}

#[test]
fn client_rejects_a_base_url_that_is_not_a_url() {
    let client = client("", None);

    assert!(matches!(
        client.resolve("/credentials"),
        Err(ClientError::Url { .. })
    ));
}

#[test]
fn client_prints_the_http_status_on_stderr_and_a_json_body_on_stdout() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    print_http_error(500, br#"{"detail":"boom"}"#, &mut stdout, &mut stderr);

    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "{\"detail\":\"boom\"}\n"
    );
    assert_eq!(String::from_utf8(stderr).unwrap(), "Error: HTTP 500\n");
}

#[test]
fn client_prints_a_non_json_error_body_as_raw_text_on_stderr() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    print_http_error(502, b"<html>nope</html>", &mut stdout, &mut stderr);

    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: HTTP 502\n<html>nope</html>\n"
    );
}
