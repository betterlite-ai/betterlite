use crate::commands::tests::support::{Answer, TestServer};

use super::super::api::ApiError;
use super::super::{KeysCommand, KeysError};
use super::support;
use super::support::{Fixture, delete, pretty, value};

#[test]
fn delete_sends_both_lists() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"deleted":2}"#)]);
    let fixture = Fixture::new(&server, Some("sk-test"));
    let mut options = delete();
    options.keys = Some(String::from("a, b"));
    options.key_aliases = Some(String::from("c,d"));

    let (result, stdout, _stderr) = support::run(KeysCommand::Delete(options), &fixture);

    result.unwrap();
    assert_eq!(stdout, pretty(&value(r#"{"deleted":2}"#)));
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /key/delete HTTP/1.1");
    assert_eq!(
        requests[0].body_text(),
        r#"{"keys":["a","b"],"key_aliases":["c","d"]}"#
    );
}

#[test]
fn delete_sends_null_for_a_list_not_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(KeysCommand::Delete(delete()), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].body_text(),
        r#"{"keys":null,"key_aliases":null}"#
    );
}

#[test]
fn delete_sends_an_empty_option_as_a_list_with_one_empty_part() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);
    let mut options = delete();
    options.keys = Some(String::new());

    let (result, _stdout, _stderr) = support::run(KeysCommand::Delete(options), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].body_text(),
        r#"{"keys":[""],"key_aliases":null}"#
    );
}

#[test]
fn delete_prints_the_status_and_the_error_body_then_aborts() {
    let server = TestServer::start(vec![Answer::json(404, r#"{"detail":"missing"}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Delete(delete()), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert_eq!(stdout, pretty(&value(r#"{"detail":"missing"}"#)));
    assert_eq!(stderr, "Error: HTTP 404\n");
}

#[test]
fn delete_passes_a_401_through() {
    let server = TestServer::start(vec![Answer::json(401, "{}")]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Delete(delete()), &fixture);

    let error = result.expect_err("a 401 is passed through");
    assert!(
        matches!(error, KeysError::Client(ApiError::Unauthorized { .. })),
        "{error:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}
