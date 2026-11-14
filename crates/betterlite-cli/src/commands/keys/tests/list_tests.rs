use rstest::rstest;

use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{Answer, TestServer};

use super::super::api::ApiError;
use super::super::{KeysCommand, KeysError, ListKeys};
use super::support;
use super::support::{Fixture, keys_page, list, pretty, query_pairs, table, value};

#[test]
fn list_sends_every_given_option_as_a_query_pair() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"keys":[]}"#)]);
    let fixture = Fixture::new(&server, Some("sk-test"));
    let command = KeysCommand::List(ListKeys {
        page: Some(2),
        size: Some(5),
        user_id: Some(String::from("u1")),
        team_id: Some(String::from("t1")),
        organization_id: Some(String::from("o1")),
        key_hash: Some(String::from("h1")),
        key_alias: Some(String::from("a1")),
        return_full_object: true,
        include_team_keys: true,
        format: OutputFormat::Table,
    });

    let (result, _stdout, _stderr) = support::run(command, &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), "GET");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(
        query_pairs(&requests[0]),
        vec![
            (String::from("page"), String::from("2")),
            (String::from("size"), String::from("5")),
            (String::from("user_id"), String::from("u1")),
            (String::from("team_id"), String::from("t1")),
            (String::from("organization_id"), String::from("o1")),
            (String::from("key_hash"), String::from("h1")),
            (String::from("key_alias"), String::from("a1")),
            (String::from("return_full_object"), String::from("true")),
            (String::from("include_team_keys"), String::from("true")),
        ]
    );
}

#[test]
fn list_sends_the_two_flags_and_no_other_query_by_default() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"keys":[]}"#)]);
    let fixture = Fixture::new(&server, None);
    let (result, _stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Json)), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        query_pairs(&requests[0]),
        vec![
            (String::from("return_full_object"), String::from("true")),
            (String::from("include_team_keys"), String::from("false")),
        ]
    );
    assert_eq!(requests[0].header("authorization"), None);
}

#[test]
fn list_parses_the_flags_with_their_click_defaults() {
    let KeysCommand::List(defaults) = support::parse(&["keys", "list"]) else {
        panic!("expected the list command");
    };
    assert!(defaults.return_full_object);
    assert!(!defaults.include_team_keys);
    assert_eq!(defaults.format, OutputFormat::Table);
    let KeysCommand::List(given) =
        support::parse(&["keys", "list", "--include-team-keys", "--page", "3"])
    else {
        panic!("expected the list command");
    };
    assert!(given.include_team_keys);
    assert_eq!(given.page, Some(3));
}

#[test]
fn list_json_prints_the_response_with_a_two_space_indent() {
    let body = r#"{"keys":[{"token":"tok"}],"total_count":1}"#;
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Json)), &fixture);

    result.unwrap();
    let expected = value(body);
    assert_eq!(value(&stdout), expected);
    assert_eq!(stdout, pretty(&expected));
    assert!(stderr.is_empty(), "{stderr}");
}

#[rstest]
#[case::table(OutputFormat::Table)]
#[case::json(OutputFormat::Json)]
fn list_rejects_a_response_that_is_not_an_object(#[case] format: OutputFormat) {
    let server = TestServer::start(vec![Answer::json(200, "[]")]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) = support::run(KeysCommand::List(list(format)), &fixture);

    let error = result.expect_err("a list response that is not an object is an error");
    let KeysError::NotAnObject { url } = &error else {
        panic!("expected a not-an-object error, got {error:?}");
    };
    assert!(url.contains("/key/list?return_full_object=true"), "{url}");
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn list_table_prints_the_showing_line_and_the_api_keys_table() {
    let body = keys_page(
        r#"{"token":"hash-1","key_alias":"main","user_id":"u1","team_id":"t1","spend":12.5},{"token":"hash-2"},{"token":"hash-3","spend":300}"#,
    );
    let server = TestServer::start(vec![Answer::json(200, &body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    result.unwrap();
    assert!(stderr.is_empty(), "{stderr}");
    let expected = format!(
        "Showing 3 keys out of 0\n{}",
        table(
            "API Keys",
            &["Key Hash", "Alias", "User ID", "Team ID", "Spend"],
            &[
                &["hash-1", "main", "u1", "t1", "12.5"],
                &["hash-2", "", "", "", ""],
                &["hash-3", "", "", "", "300"],
            ]
        )
    );
    assert_eq!(stdout, expected);
}

#[test]
fn list_table_reads_a_null_keys_list_and_total_count_as_empty_and_zero() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"keys":null,"total_count":null}"#,
    )]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    result.unwrap();
    let expected = format!(
        "Showing 0 keys out of 0\n{}",
        table(
            "API Keys",
            &["Key Hash", "Alias", "User ID", "Team ID", "Spend"],
            &[]
        )
    );
    assert_eq!(stdout, expected);
}

#[test]
fn list_table_prints_the_total_count_as_given() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"keys":[{"token":"tok"}],"total_count":7}"#,
    )]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    result.unwrap();
    assert!(stdout.starts_with("Showing 1 keys out of 7\n"), "{stdout}");
}

#[test]
fn list_table_names_a_key_field_of_another_type() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"keys":[{"spend":"free"}]}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    let error = result.expect_err("a string spend is an error");
    let KeysError::Field { field, .. } = &error else {
        panic!("expected a field error, got {error:?}");
    };
    assert_eq!(field, "keys[0].spend");
    assert!(error.to_string().contains("keys[0].spend"), "{error}");
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn list_table_names_a_total_count_of_another_type() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"keys":[],"total_count":"many"}"#,
    )]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    let error = result.expect_err("a string total_count is an error");
    let KeysError::Field { field, .. } = &error else {
        panic!("expected a field error, got {error:?}");
    };
    assert_eq!(field, "total_count");
}

#[test]
fn list_table_names_a_key_entry_that_is_not_an_object() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"keys":[5]}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    let error = result.expect_err("an entry that is not an object is an error");
    let KeysError::Field { field, .. } = &error else {
        panic!("expected a field error, got {error:?}");
    };
    assert_eq!(field, "keys[0]");
}

#[test]
fn list_passes_a_client_error_through_without_printing() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    let error = result.expect_err("an HTTP error is passed through");
    assert!(
        matches!(error, KeysError::Client(ApiError::Http { status: 500, .. })),
        "{error:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn list_turns_a_401_into_unauthorized_with_the_redacted_text() {
    let server = TestServer::start(vec![Answer::json(401, r#"{"detail":"unauthorized"}"#)]);
    let fixture = Fixture::new(&server, None);
    let mut options = list(OutputFormat::Table);
    options.key_hash = Some(String::from("sk-abcdefghijklmnopqrst"));

    let (result, stdout, stderr) = support::run(KeysCommand::List(options), &fixture);

    let error = result.expect_err("a 401 is unauthorized");
    let KeysError::Client(ApiError::Unauthorized { text }) = &error else {
        panic!("expected an unauthorized error, got {error:?}");
    };
    assert!(text.contains("REDACTED"), "{text}");
    assert!(!text.contains("sk-abcdefghijklmnopqrst"), "{text}");
    assert!(
        text.starts_with("HTTP 401 Unauthorized for GET http://"),
        "{text}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn list_names_the_url_when_the_body_is_not_json() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Json)), &fixture);

    let error = result.expect_err("a body that is not JSON is an error");
    let KeysError::Client(ApiError::Response { url, .. }) = &error else {
        panic!("expected a response error, got {error:?}");
    };
    assert!(url.contains("/key/list?return_full_object=true"), "{url}");
}
