use rstest::rstest;

use crate::commands::tests::support::{Answer, TestServer};

use super::super::{KeysCommand, KeysError};
use super::support;
use super::support::{Fixture, api_key, body_json, keys_page, pretty, table, value};

#[test]
fn import_pages_the_source_at_one_hundred_and_stops_on_a_short_page() {
    let first = keys_page(&vec![r#"{"key_alias":"k"}"#; 100].join(","));
    let second = keys_page(r#"{"key_alias":"last"}"#);
    let source = TestServer::start(vec![Answer::json(200, &first), Answer::json(200, &second)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, Some("sk-dest"));
    let mut options = support::import(&source.base_url);
    options.dry_run = true;

    let (result, stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(stderr.is_empty(), "{stderr}");
    let header = format!("Fetching keys from source server: {}\n", source.base_url);
    assert!(stdout.starts_with(&header), "{stdout}");
    assert!(stdout.contains("Fetched page 1: 100 keys"), "{stdout}");
    assert!(stdout.contains("Fetched page 2: 1 keys"), "{stdout}");
    assert!(
        stdout.contains("Found 101 keys in source instance."),
        "{stdout}"
    );
    assert!(stdout.contains("\n--- DRY RUN MODE ---\n"), "{stdout}");
    assert!(stdout.contains("Keys that would be imported"), "{stdout}");
    let requests = source.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path(),
        "/key/list?page=1&size=100&return_full_object=true"
    );
    assert_eq!(
        requests[1].path(),
        "/key/list?page=2&size=100&return_full_object=true"
    );
    assert!(destination.finish().is_empty());
}

#[rstest]
#[case::empty_list(r#"{"keys":[]}"#)]
#[case::null_list(r#"{"keys":null}"#)]
#[case::absent_list(r#"{"success":true}"#)]
fn import_stops_when_a_page_has_no_keys(#[case] body: &str) {
    let source = TestServer::start(vec![Answer::json(200, body)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.dry_run = true;

    let (result, stdout, _stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(
        stdout.ends_with("No keys found in source instance.\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("Fetched page"), "{stdout}");
    assert_eq!(source.finish().len(), 1);
}

#[test]
fn import_names_a_page_field_of_another_type() {
    let source = TestServer::start(vec![Answer::json(200, r#"{"keys":[{"spend":"free"}]}"#)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.dry_run = true;

    let (result, stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert!(
        stderr.starts_with("Error: the field keys[0].spend of page 1 is not valid: "),
        "{stderr}"
    );
    assert!(
        stdout.contains("Fetching keys from source server:"),
        "{stdout}"
    );
}

#[test]
fn import_names_a_page_that_is_not_an_object() {
    let source = TestServer::start(vec![Answer::json(200, "[]")]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.dry_run = true;

    let (result, _stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert_eq!(
        stderr,
        "Error: the response for page 1 is not a JSON object\n"
    );
}

#[test]
fn import_filters_keys_by_created_since_and_drops_the_offset() {
    let page = keys_page(
        r#"{"key_alias":"dropped-wall","created_at":"2024-05-01T23:30:00-02:00"},{"key_alias":"kept-wall","created_at":"2024-05-01T23:50:00-02:00"},{"key_alias":"dropped-utc","created_at":"2024-05-01T12:00:00Z"},{"key_alias":"dropped-absent"},{"key_alias":"dropped-null","created_at":null}"#,
    );
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.dry_run = true;
    options.created_since = Some(String::from("2024-05-01_23:45"));

    let (result, stdout, _stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(
        stdout.contains("Filtered 5 keys to 1 keys created since 2024-05-01_23:45"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Found 1 keys in source instance."),
        "{stdout}"
    );
    assert!(stdout.contains("kept-wall"), "{stdout}");
    assert!(!stdout.contains("dropped-wall"), "{stdout}");
    assert!(!stdout.contains("dropped-utc"), "{stdout}");
    assert!(!stdout.contains("dropped-absent"), "{stdout}");
    assert!(!stdout.contains("dropped-null"), "{stdout}");
}

#[test]
fn import_prints_no_keys_when_the_filter_removes_every_key() {
    let page = keys_page(r#"{"key_alias":"a","created_at":"2020-01-01T00:00:00"}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.created_since = Some(String::from("2024-01-01"));

    let (result, stdout, _stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(
        stdout.contains("Filtered 1 keys to 0 keys created since 2024-01-01"),
        "{stdout}"
    );
    assert!(
        stdout.contains("No keys found in source instance."),
        "{stdout}"
    );
    assert!(!stdout.contains("Found"), "{stdout}");
}

#[rstest]
#[case::empty("")]
#[case::underscore_without_minutes("2024-05-01_10")]
#[case::impossible_month("2024-13-01")]
#[case::text("yesterday")]
fn import_rejects_a_bad_created_since_before_any_request(#[case] created_since: &str) {
    let source = TestServer::start(Vec::new());
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.created_since = Some(created_since.to_owned());

    let (result, stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert_eq!(
        stderr,
        format!(
            "Error: Invalid date format '{created_since}'. Use YYYY-MM-DD_HH:MM or YYYY-MM-DD\n"
        )
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(source.finish().is_empty());
    assert!(destination.finish().is_empty());
}

#[test]
fn import_prints_the_dry_run_table_with_the_created_wall_time() {
    let page = keys_page(
        r#"{"key_alias":"main","user_id":"u1","created_at":"2024-05-01T23:30:00.123456Z"}"#,
    );
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.dry_run = true;

    let (result, stdout, _stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    let expected = format!(
        "Fetching keys from source server: {}\nFetched page 1: 1 keys\nFound 1 keys in source instance.\n\n--- DRY RUN MODE ---\n{}",
        source.base_url,
        table(
            "Keys that would be imported",
            &["Key Alias", "User ID", "Created"],
            &[&["main", "u1", "2024-05-01 23:30"]]
        )
    );
    assert_eq!(stdout, expected);
    assert!(destination.finish().is_empty());
}

#[test]
fn import_copies_every_present_field_to_the_destination() {
    let page = keys_page(
        r#"{"models":[],"aliases":{},"spend":0,"key_alias":"","team_id":"","user_id":"","budget_id":"","config":{},"created_at":"2024-05-01T00:00:00"}"#,
    );
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(vec![Answer::json(200, r#"{"key":"sk-new"}"#)]);
    let fixture = Fixture::new(&destination, Some("sk-dest"));
    let mut options = support::import(&source.base_url);
    options.source_api_key = Some(api_key("sk-source"));

    let (result, _stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(stderr.is_empty(), "{stderr}");
    let source_requests = source.finish();
    assert_eq!(
        source_requests[0].header("authorization"),
        Some("Bearer sk-source")
    );
    let destination_requests = destination.finish();
    assert_eq!(destination_requests.len(), 1);
    assert_eq!(
        destination_requests[0].header("authorization"),
        Some("Bearer sk-dest")
    );
    assert_eq!(
        destination_requests[0].request_line,
        "POST /key/generate HTTP/1.1"
    );
    assert_eq!(
        body_json(&destination_requests[0]),
        value(
            r#"{"models":[],"aliases":{},"spend":0,"key_alias":"","team_id":"","user_id":"","budget_id":"","config":{}}"#
        )
    );
}

#[test]
fn import_omits_absent_and_null_fields() {
    let page = keys_page(r#"{"key_alias":"a","team_id":null,"models":null,"spend":null}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, _stdout, _stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    let destination_requests = destination.finish();
    assert_eq!(
        body_json(&destination_requests[0]),
        value(r#"{"key_alias":"a"}"#)
    );
}

#[test]
fn import_prints_the_generated_and_imported_lines_and_the_summary() {
    let page = keys_page(r#"{"key_alias":"a"},{"key_alias":"b"}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(vec![
        Answer::json(200, r#"{"key":"sk-1"}"#),
        Answer::json(200, r#"{"key":"sk-2"}"#),
    ]);
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    let expected = format!(
        "Fetching keys from source server: {}\nFetched page 1: 2 keys\nFound 2 keys in source instance.\nGenerated key: {}\nImported key: a\nGenerated key: {}\nImported key: b\n\nImport completed:\n  Successfully imported: 2\n  Failed to import: 0\n  Total keys processed: 2\n",
        source.base_url, r#"{"key":"sk-1"}"#, r#"{"key":"sk-2"}"#
    );
    assert_eq!(stdout, expected);
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn import_counts_a_failed_key_and_continues() {
    let page = keys_page(r#"{"key_alias":"a"},{"key_alias":"b"}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(vec![
        Answer::json(500, r#"{"detail":"boom"}"#),
        Answer::json(200, r#"{"key":"sk-2"}"#),
    ]);
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert_eq!(
        stderr,
        format!(
            "Failed to import key a: HTTP 500 Internal Server Error for POST {}/key/generate\n",
            destination.base_url
        )
    );
    assert!(stdout.contains("Imported key: b\n"), "{stdout}");
    assert!(
        stdout.ends_with(
            "  Successfully imported: 1\n  Failed to import: 1\n  Total keys processed: 2\n"
        ),
        "{stdout}"
    );
}

#[test]
fn import_reports_a_destination_401_as_a_failed_key() {
    let page = keys_page(r#"{"key_alias":"a"}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(vec![Answer::json(401, "{}")]);
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, _stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(
        stderr.starts_with("Failed to import key a: HTTP 401 Unauthorized for POST "),
        "{stderr}"
    );
    assert!(!stderr.contains("Error: HTTP"), "{stderr}");
}

#[test]
fn import_prints_the_status_and_the_source_error_body_then_aborts() {
    let source = TestServer::start(vec![Answer::json(500, r#"{"detail":"down"}"#)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert_eq!(
        stdout,
        format!(
            "Fetching keys from source server: {}\n{}",
            source.base_url,
            pretty(&value(r#"{"detail":"down"}"#))
        )
    );
    assert_eq!(stderr, "Error: HTTP 500\n");
}

#[test]
fn import_prints_a_source_401_as_one_error_line() {
    let source = TestServer::start(vec![Answer::json(401, "{}")]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, _stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert!(
        stderr.starts_with("Error: HTTP 401 Unauthorized for GET "),
        "{stderr}"
    );
    assert!(
        stderr.contains("/key/list?page=1&size=100&return_full_object=true"),
        "{stderr}"
    );
}

#[test]
fn import_prints_a_bad_source_body_as_one_error_line() {
    let source = TestServer::start(vec![Answer::text(200, "not json")]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, _stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert!(stderr.starts_with("Error: the response from "), "{stderr}");
    assert!(
        stderr.contains("/key/list?page=1&size=100&return_full_object=true"),
        "{stderr}"
    );
}

#[test]
fn import_names_created_at_when_the_dry_run_table_cannot_read_it() {
    let page = keys_page(r#"{"key_alias":"a","created_at":"yesterday"}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(Vec::new());
    let fixture = Fixture::new(&destination, None);
    let mut options = support::import(&source.base_url);
    options.dry_run = true;

    let (result, _stdout, stderr) = support::run(KeysCommand::Import(options), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert!(
        stderr.starts_with("Error: the created_at value 'yesterday' is not a date-time: "),
        "{stderr}"
    );
}
