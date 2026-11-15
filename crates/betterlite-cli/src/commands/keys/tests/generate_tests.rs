use std::net::TcpListener;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::commands::tests::support::{Answer, TestServer};

use super::super::api::ApiError;
use super::super::{KeysCommand, KeysError, UsageError};
use super::support;
use super::support::{Fixture, body_json, generate, pretty, value};

#[test]
fn generate_sends_only_the_given_fields_in_the_client_order() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"key":"sk-new"}"#)]);
    let fixture = Fixture::new(&server, Some("sk-test"));
    let mut options = generate();
    options.models = Some(String::from("gpt-4, gpt-3.5-turbo"));
    options.aliases = Some(String::from(r#"{"fast":"gpt-4"}"#));
    options.spend = Some(Decimal::new(25, 1));
    options.duration = Some(String::from("24h"));
    options.key_alias = Some(String::from("main"));
    options.team_id = Some(String::from("team"));
    options.user_id = Some(String::from("user"));
    options.budget_id = Some(String::from("budget"));
    options.config = Some(String::from(r#"{"a":1}"#));

    let (result, stdout, stderr) = support::run(KeysCommand::Generate(options), &fixture);

    result.unwrap();
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(stdout, pretty(&value(r#"{"key":"sk-new"}"#)));
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), "POST");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(
        requests[0].body_text(),
        r#"{"models":["gpt-4","gpt-3.5-turbo"],"aliases":{"fast":"gpt-4"},"spend":2.5,"duration":"24h","key_alias":"main","team_id":"team","user_id":"user","budget_id":"budget","config":{"a":1}}"#
    );
}

#[test]
fn generate_sends_an_empty_object_when_no_option_is_given() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"key":"sk-new"}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, _stdout, _stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /key/generate HTTP/1.1");
    assert_eq!(requests[0].body_text(), "{}");
}

#[rstest]
#[case::parts_are_trimmed(" a , b ", r#"["a","b"]"#)]
#[case::empty_parts_stay("a,,b", r#"["a","","b"]"#)]
#[case::empty_models("", r#"[""]"#)]
fn generate_splits_models_on_commas_and_trims_each_part(
    #[case] models: &str,
    #[case] expected: &str,
) {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);
    let mut options = generate();
    options.models = Some(models.to_owned());

    let (result, _stdout, _stderr) = support::run(KeysCommand::Generate(options), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        body_json(&requests[0]),
        value(&format!(r#"{{"models":{expected}}}"#))
    );
}

#[test]
fn generate_reads_aliases_and_config_as_objects() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);
    let mut options = generate();
    options.aliases = Some(String::from(r#"{"a":"b"}"#));
    options.config = Some(String::from(r#"{"nested":{"n":1},"flag":true}"#));

    let (result, _stdout, _stderr) = support::run(KeysCommand::Generate(options), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        body_json(&requests[0]),
        value(r#"{"aliases":{"a":"b"},"config":{"nested":{"n":1},"flag":true}}"#)
    );
}

#[test]
fn generate_uses_an_empty_option_as_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);
    let mut options = generate();
    options.models = Some(String::new());
    options.duration = Some(String::new());
    options.key_alias = Some(String::new());

    let (result, _stdout, _stderr) = support::run(KeysCommand::Generate(options), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].body_text(),
        r#"{"models":[""],"duration":"","key_alias":""}"#
    );
}

#[test]
fn generate_sends_a_spend_of_zero() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);
    let fixture = Fixture::new(&server, None);
    let mut options = generate();
    options.spend = Some(Decimal::ZERO);

    let (result, _stdout, _stderr) = support::run(KeysCommand::Generate(options), &fixture);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].body_text(), r#"{"spend":0}"#);
}

#[rstest]
#[case::aliases("not json", "{}")]
#[case::config("{}", "not json")]
fn generate_rejects_invalid_json_before_any_request(#[case] aliases: &str, #[case] config: &str) {
    let server = TestServer::start(Vec::new());
    let fixture = Fixture::new(&server, None);
    let mut options = generate();
    options.aliases = Some(aliases.to_owned());
    options.config = Some(config.to_owned());

    let (result, stdout, _stderr) = support::run(KeysCommand::Generate(options), &fixture);

    let error = result.expect_err("invalid JSON is a usage error");
    let KeysError::Usage(UsageError::InvalidJson { .. }) = &error else {
        panic!("expected a usage error, got {error:?}");
    };
    assert!(error.to_string().starts_with("Invalid JSON: "), "{error}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(server.finish().is_empty());
}

#[rstest]
#[case::aliases_list("[]", "{}", "aliases")]
#[case::aliases_non_string(r#"{"a":1}"#, "{}", "aliases")]
#[case::config_list("{}", "[]", "config")]
fn generate_rejects_json_of_another_type_by_option_name(
    #[case] aliases: &str,
    #[case] config: &str,
    #[case] option: &str,
) {
    let server = TestServer::start(Vec::new());
    let fixture = Fixture::new(&server, None);
    let mut options = generate();
    options.aliases = Some(aliases.to_owned());
    options.config = Some(config.to_owned());

    let (result, _stdout, _stderr) = support::run(KeysCommand::Generate(options), &fixture);

    let error = result.expect_err("another JSON type is a usage error");
    let KeysError::Usage(UsageError::OptionType { option: named, .. }) = &error else {
        panic!("expected a usage error, got {error:?}");
    };
    assert_eq!(*named, option);
    assert!(error.to_string().contains(option), "{error}");
    assert!(server.finish().is_empty());
}

#[test]
fn generate_prints_the_status_and_a_json_body_then_aborts() {
    let server = TestServer::start(vec![Answer::json(422, r#"{"detail":"bad"}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert_eq!(stdout, pretty(&value(r#"{"detail":"bad"}"#)));
    assert_eq!(stderr, "Error: HTTP 422\n");
}

#[test]
fn generate_prints_a_non_json_error_body_to_stderr() {
    let server = TestServer::start(vec![Answer::text(500, "the proxy broke")]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    assert!(matches!(result, Err(KeysError::Aborted)), "{result:?}");
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(stderr, "Error: HTTP 500\nthe proxy broke\n");
}

#[test]
fn generate_passes_a_401_through_without_an_http_line() {
    let server = TestServer::start(vec![Answer::json(401, r#"{"detail":"unauthorized"}"#)]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    let error = result.expect_err("a 401 is passed through");
    assert!(
        matches!(error, KeysError::Client(ApiError::Unauthorized { .. })),
        "{error:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn generate_passes_a_transport_error_through() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let fixture = Fixture::at(&base_url, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    let error = result.expect_err("a refused connection is passed through");
    assert!(
        matches!(error, KeysError::Client(ApiError::Client(_))),
        "{error:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn generate_passes_a_json_error_through() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);
    let fixture = Fixture::new(&server, None);

    let (result, stdout, stderr) = support::run(KeysCommand::Generate(generate()), &fixture);

    let error = result.expect_err("a body that is not JSON is passed through");
    assert!(
        matches!(error, KeysError::Client(ApiError::Response { .. })),
        "{error:?}"
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}
