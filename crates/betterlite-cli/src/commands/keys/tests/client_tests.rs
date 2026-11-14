use std::time::Duration;

use http::Method;
use rstest::rstest;

use crate::commands::REQUEST_TIMEOUT;
use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{Answer, TestServer};

use super::super::api::{ApiError, Reply, interpret};
use super::super::{KeysCommand, KeysError};
use super::support;
use super::support::{Fixture, list};

#[test]
fn client_strips_every_trailing_slash_from_the_base_url() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::at(&format!("{}///", server.base_url), None);

    let (result, _stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Json)), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].request_line,
        "GET /key/list?return_full_object=true&include_team_keys=false HTTP/1.1"
    );
}

#[rstest]
#[case::with_a_key(Some("sk-test"), Some("Bearer sk-test"))]
#[case::without_a_key(None, None)]
fn client_adds_bearer_only_when_a_key_is_given(
    #[case] key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, key);

    let (result, _stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Json)), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].header("authorization"), expected);
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
}

#[test]
fn client_uses_the_thirty_second_request_timeout() {
    assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(30));
}

#[test]
fn client_reports_the_status_reason_method_and_url() {
    let reply = Reply {
        method: Method::POST,
        url: String::from("http://proxy.test/key/generate"),
        status: 422,
        body: b"{}".to_vec(),
    };

    let error = interpret(&reply, None).expect_err("a 422 is an HTTP error");

    assert_eq!(
        error.to_string(),
        "HTTP 422 Unprocessable Entity for POST http://proxy.test/key/generate"
    );
}

#[test]
fn client_redacts_the_401_text() {
    let server = TestServer::start(vec![Answer::json(401, "{}")]);
    let fixture = Fixture::new(&server, Some("sk-plain"));
    let mut options = list(OutputFormat::Json);
    options.user_id = Some(String::from("sk-abcdefghijklmnopqrst"));

    let (result, _stdout, _stderr) = support::run(KeysCommand::List(options), &fixture);

    let error = result.expect_err("a 401 is unauthorized");
    let KeysError::Client(ApiError::Unauthorized { text }) = &error else {
        panic!("expected an unauthorized error, got {error:?}");
    };
    assert!(text.contains("user_id=REDACTED"), "{text}");
}
