#![cfg(test)]

pub(super) mod support;

use rstest::rstest;

use crate::commands::keys::api::ApiError;
use crate::commands::tests::support::{Answer, TestServer};

use super::{EncryptionCommand, EncryptionError};

use support::{Fixture, Flags, pretty, value};

#[test]
fn migrate_check_sends_a_get_without_a_content_type() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"status":"success"}"#)]);
    let fixture = Fixture::new(&server, Some("sk-test"));

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request_line,
        "GET /credentials/migrate-encryption/check HTTP/1.1"
    );
    assert_eq!(requests[0].header("content-type"), None);
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
}

#[test]
fn migrate_sends_a_post_with_an_empty_json_body() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::None)),
        &fixture,
    );

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].request_line,
        "POST /credentials/migrate-encryption HTTP/1.1"
    );
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(requests[0].body_text(), "{}");
}

#[test]
fn migrate_dry_run_sends_the_dry_run_query_parameter() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::DryRun)),
        &fixture,
    );

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        support::query_pairs(&requests[0]),
        vec![(String::from("dry_run"), String::from("true"))]
    );
    assert_eq!(requests[0].body_text(), "{}");
}

#[test]
fn migrate_check_wins_when_both_flags_are_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Both)),
        &fixture,
    );

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request_line,
        "GET /credentials/migrate-encryption/check HTTP/1.1"
    );
    assert_eq!(requests[0].header("content-type"), None);
}

#[test]
fn migrate_prints_the_response_pretty() {
    let body = r#"{"status":"success","dry_run":false,"report":{"residual_legacy":0}}"#;
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::None)),
        &fixture,
    );

    result.unwrap();
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(
        stdout,
        format!(
            "{}No legacy values remaining (residual_legacy == 0).\n",
            pretty(&value(body))
        )
    );
}

#[test]
fn migrate_prints_the_residual_line_above_zero() {
    let body = r#"{"report":{"residual_legacy":3}}"#;
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    result.unwrap();
    assert_eq!(
        stdout,
        format!(
            "{}Residual legacy values remaining: 3\n",
            pretty(&value(body))
        )
    );
}

#[rstest]
#[case::absent(r#"{"status":"success","report":{}}"#)]
#[case::null_value(r#"{"report":{"residual_legacy":null}}"#)]
#[case::negative(r#"{"report":{"residual_legacy":-1}}"#)]
#[case::absent_report(r#"{"status":"success"}"#)]
#[case::null_report(r#"{"report":null}"#)]
fn migrate_prints_nothing_for_an_absent_null_or_negative_residual(#[case] body: &str) {
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    result.unwrap();
    assert_eq!(stdout, pretty(&value(body)));
}

#[test]
fn migrate_rejects_a_response_that_is_not_an_object() {
    let server = TestServer::start(vec![Answer::json(200, "[]")]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    let error = result.expect_err("a response that is not an object is an error");
    let EncryptionError::NotAnObject { url } = &error else {
        panic!("expected a not-an-object error, got {error:?}");
    };
    assert!(
        url.ends_with("/credentials/migrate-encryption/check"),
        "{url}"
    );
    assert_eq!(stdout, pretty(&value("[]")));
}

#[test]
fn migrate_rejects_a_report_that_is_not_an_object() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"report":[]}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    let error = result.expect_err("a report that is not an object is an error");
    let EncryptionError::ReplyField { field, .. } = &error else {
        panic!("expected a reply field error, got {error:?}");
    };
    assert_eq!(field, "report");
}

#[rstest]
#[case::text(r#"{"report":{"residual_legacy":"3"}}"#)]
#[case::float(r#"{"report":{"residual_legacy":1.5}}"#)]
fn migrate_rejects_a_residual_that_is_not_an_integer(#[case] body: &str) {
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    let error = result.expect_err("a residual that is not an integer is an error");
    let EncryptionError::ReplyField { field, .. } = &error else {
        panic!("expected a reply field error, got {error:?}");
    };
    assert_eq!(field, "report.residual_legacy");
}

#[test]
fn migrate_passes_an_http_error_through_unchanged() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::None)),
        &fixture,
    );

    let error = result.expect_err("an HTTP error is passed through");
    assert!(
        matches!(
            error,
            EncryptionError::Api(ApiError::Http { status: 500, .. })
        ),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!(
            "HTTP 500 Internal Server Error for POST {}/credentials/migrate-encryption",
            server.base_url
        )
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn migrate_does_not_turn_a_401_into_unauthorized() {
    let server = TestServer::start(vec![Answer::json(401, "{}")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    let error = result.expect_err("a 401 is an HTTP error here");
    assert!(
        matches!(
            error,
            EncryptionError::Api(ApiError::Http { status: 401, .. })
        ),
        "{error:?}"
    );
}

#[test]
fn migrate_names_the_url_when_the_body_is_not_json() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(
        EncryptionCommand::Migrate(support::migrate(Flags::Check)),
        &fixture,
    );

    let error = result.expect_err("a body that is not JSON is an error");
    let EncryptionError::Api(ApiError::Response { url, .. }) = &error else {
        panic!("expected a response error, got {error:?}");
    };
    assert!(
        url.ends_with("/credentials/migrate-encryption/check"),
        "{url}"
    );
}
