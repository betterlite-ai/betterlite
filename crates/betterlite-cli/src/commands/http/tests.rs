#![cfg(test)]

use super::{HttpCommand, HttpError, RequestArgs, run_http_command};
use crate::cli_context::CliContext;
use crate::commands::ClientError;
use crate::commands::tests::support::{Answer, TestServer, client, client_for};

fn options(method: &str, uri: &str) -> RequestArgs {
    RequestArgs {
        method: method.to_owned(),
        uri: uri.to_owned(),
        data: None,
        json: None,
        header: Vec::new(),
    }
}

fn run_against(
    server: &TestServer,
    request: RequestArgs,
) -> (Result<(), HttpError>, String, String) {
    run_with_key(server, request, Some("sk-test"))
}

fn run_with_key(
    server: &TestServer,
    request: RequestArgs,
    api_key: Option<&str>,
) -> (Result<(), HttpError>, String, String) {
    let api = client_for(server, api_key);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_http_command(
        HttpCommand::Request(request),
        &CliContext::default(),
        &api,
        &mut stdout,
        &mut stderr,
    );
    (
        result,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn request_build_headers_split_on_the_first_colon_and_trim() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("GET", "/credentials");
    request.header = vec![
        String::from("X-Custom: value:with:colons"),
        String::from("x-lower:  v  "),
    ];

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].header("x-custom"), Some("value:with:colons"));
    assert_eq!(requests[0].header("x-lower"), Some("v"));
}

#[test]
fn request_build_rejects_a_header_without_a_colon() {
    let server = TestServer::start(Vec::new());
    let mut request = options("GET", "/credentials");
    request.header = vec![String::from("nocolon")];

    let (result, stdout, stderr) = run_against(&server, request);

    let Err(HttpError::HeaderFormat { header }) = result else {
        panic!("expected a header format error");
    };
    assert_eq!(header, "nocolon");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    assert!(server.finish().is_empty());
}

#[test]
fn request_build_json_is_sent_as_the_body() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("POST", "/credentials");
    request.json = Some(String::from(r#"{"model":"gpt-4o"}"#));

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].method(), "POST");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(requests[0].body_text(), r#"{"model":"gpt-4o"}"#);
}

#[test]
fn request_build_keeps_a_caller_content_type() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("POST", "/credentials");
    request.json = Some(String::from(r#"{"a":1}"#));
    request.header = vec![String::from("Content-Type: application/merge-patch+json")];

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].header("content-type"),
        Some("application/merge-patch+json")
    );
    assert_eq!(requests[0].body_text(), r#"{"a":1}"#);
}

#[test]
fn request_build_data_is_tried_as_json_first() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("POST", "/credentials");
    request.data = Some(String::from(r#"{"a":1}"#));

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(requests[0].body_text(), r#"{"a":1}"#);
}

#[test]
fn request_build_data_falls_back_to_a_raw_string() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("POST", "/credentials");
    request.data = Some(String::from("hello world"));

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].header("content-type"), None);
    assert_eq!(requests[0].body_text(), "hello world");
}

#[test]
fn request_build_json_wins_over_data() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("POST", "/credentials");
    request.json = Some(String::from(r#"{"j":1}"#));
    request.data = Some(String::from(r#"{"d":2}"#));

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].body_text(), r#"{"j":1}"#);
}

#[test]
fn request_build_uri_joins_with_every_leading_slash_removed() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, _stdout, _stderr) = run_against(&server, options("GET", "//credentials"));

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /credentials HTTP/1.1");
}

#[test]
fn request_build_user_authorization_is_overwritten_by_the_key() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("GET", "/credentials");
    request.header = vec![String::from("Authorization: Bearer user")];

    let (result, _stdout, _stderr) = run_against(&server, request);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
}

#[test]
fn request_build_keeps_the_user_authorization_without_a_key() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let mut request = options("GET", "/credentials");
    request.header = vec![String::from("Authorization: Bearer user")];

    let (result, _stdout, _stderr) = run_with_key(&server, request, None);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].header("authorization"), Some("Bearer user"));
}

#[test]
fn request_build_uppercases_the_method() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, _stdout, _stderr) = run_against(&server, options("post", "/credentials"));

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /credentials HTTP/1.1");
}

#[test]
fn request_build_success_prints_the_response_as_pretty_json() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"b":1,"a":[1,2]}"#)]);

    let (result, stdout, stderr) = run_against(&server, options("GET", "/credentials"));

    result.unwrap();
    assert_eq!(
        stdout,
        "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": 1\n}\n"
    );
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn request_error_prints_the_http_status_and_the_json_body() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);

    let (result, stdout, stderr) = run_against(&server, options("GET", "/credentials"));

    assert!(matches!(result, Err(HttpError::Aborted)));
    assert_eq!(stdout, "{\"detail\":\"boom\"}\n");
    assert_eq!(stderr, "Error: HTTP 500\n");
}

#[test]
fn request_error_prints_a_raw_text_body() {
    let server = TestServer::start(vec![Answer::text(502, "bad gateway")]);

    let (result, stdout, stderr) = run_against(&server, options("GET", "/credentials"));

    assert!(matches!(result, Err(HttpError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(stderr, "Error: HTTP 502\nbad gateway\n");
}

#[test]
fn request_error_reports_an_invalid_json_flag() {
    let server = TestServer::start(Vec::new());
    let mut request = options("POST", "/credentials");
    request.json = Some(String::from("{"));

    let (result, _stdout, stderr) = run_against(&server, request);

    let Err(error) = result else {
        panic!("expected an invalid JSON error");
    };
    assert!(
        error.to_string().starts_with("Invalid JSON format: "),
        "{error}"
    );
    assert!(stderr.is_empty(), "{stderr}");
    assert!(server.finish().is_empty());
}

#[test]
fn request_error_reports_a_bad_method() {
    let server = TestServer::start(Vec::new());

    let (result, _stdout, _stderr) = run_against(&server, options("GE T", "/credentials"));

    let Err(error) = result else {
        panic!("expected a method error");
    };
    assert_eq!(error.to_string(), "GE T is not a valid HTTP method");
    assert!(server.finish().is_empty());
}

#[test]
fn request_error_reports_a_transport_failure() {
    let api = client("http://127.0.0.1:1", None);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let result = run_http_command(
        HttpCommand::Request(options("GET", "/credentials")),
        &CliContext::default(),
        &api,
        &mut stdout,
        &mut stderr,
    );

    assert!(
        matches!(result, Err(HttpError::Client(ClientError::Transport(_)))),
        "{result:?}"
    );
}

#[test]
fn request_error_reports_a_non_json_success_body() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);

    let (result, stdout, _stderr) = run_against(&server, options("GET", "/credentials"));

    let Err(error) = result else {
        panic!("expected a response error");
    };
    assert_eq!(error.to_string(), "the response body is not valid JSON");
    assert!(stdout.is_empty(), "{stdout}");
}
