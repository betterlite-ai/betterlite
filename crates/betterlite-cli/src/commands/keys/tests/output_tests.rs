use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{Answer, TestServer};

use super::super::{KeysCommand, KeysError};
use super::support;
use super::support::{Fixture, generate, keys_page, list, value};

#[test]
fn output_prints_standard_json_with_a_two_space_indent_and_a_newline() {
    let body = r#"{"key":"sk-new","nested":{"n":1},"list":[1,2]}"#;
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    result.unwrap();
    let expected = value(body);
    assert_eq!(value(&stdout), expected);
    assert_eq!(
        stdout,
        format!("{}\n", serde_json::to_string_pretty(&expected).unwrap())
    );
    assert!(stdout.contains("  \"key\": \"sk-new\""), "{stdout}");
}

#[test]
fn output_prints_a_generated_key_as_one_line_json() {
    let page = keys_page(r#"{"key_alias":"a"}"#);
    let source = TestServer::start(vec![Answer::json(200, &page)]);
    let destination = TestServer::start(vec![Answer::json(200, r#"{"key":"sk-1","n":1}"#)]);
    let fixture = Fixture::new(&destination, None);
    let options = support::import(&source.base_url);

    let (result, stdout, _stderr) = support::run(KeysCommand::Import(options), &fixture);

    result.unwrap();
    assert!(
        stdout.contains("Generated key: {\"key\":\"sk-1\",\"n\":1}\n"),
        "{stdout}"
    );
}

#[test]
fn output_prints_a_json_error_body_pretty() {
    let body = r#"{"error":{"message":"bad","code":"422"}}"#;
    let server = TestServer::start(vec![Answer::json(422, body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert_eq!(
        stdout,
        format!("{}\n", serde_json::to_string_pretty(&value(body)).unwrap())
    );
}

#[test]
fn output_renders_strings_numbers_and_empty_cells_as_plain_text() {
    let body = keys_page(r#"{"key_alias":"alias","spend":7}"#);
    let server = TestServer::start(vec![Answer::json(200, &body)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, _stderr) =
        support::run(KeysCommand::List(list(OutputFormat::Table)), &fixture);

    result.unwrap();
    assert!(stdout.contains("alias"), "{stdout}");
    assert!(stdout.contains(" 7 "), "{stdout}");
    assert!(!stdout.contains('\u{1b}'), "{stdout}");
}
