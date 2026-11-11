#![cfg(test)]

use rstest::rstest;

use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{Answer, TestServer, client_for};

use super::{ListModelGroups, ModelGroupsCommand, ModelGroupsError, run_model_groups_command};

fn run(format: OutputFormat, server: &TestServer) -> (Result<(), ModelGroupsError>, String) {
    let api = client_for(server, Some("sk-test"));
    let mut stdout = Vec::new();
    let result = run_model_groups_command(
        ModelGroupsCommand::List(ListModelGroups { format }),
        &api,
        &mut stdout,
    );
    (result, String::from_utf8(stdout).unwrap())
}

#[test]
fn list_prints_the_accessible_model_groups_table() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_group":"gpt-4o","mode":"chat","input_cost_per_token":0.5,"output_cost_per_token":0.25},{"model_group":"embed"}]}"#,
    )]);

    let (result, stdout) = run(OutputFormat::Table, &server);

    result.unwrap();
    assert!(stdout.contains("Accessible Model Groups"), "{stdout}");
    assert!(stdout.contains("Model"), "{stdout}");
    assert!(stdout.contains("Mode"), "{stdout}");
    assert!(stdout.contains("Input $/token"), "{stdout}");
    assert!(stdout.contains("Output $/token"), "{stdout}");
    assert!(stdout.contains("gpt-4o"), "{stdout}");
    assert!(stdout.contains("0.5"), "{stdout}");
    assert!(stdout.contains("0.25"), "{stdout}");
    assert!(stdout.contains("embed"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /model_group/info HTTP/1.1");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(requests[0].header("content-type"), None);
}

#[test]
fn list_json_prints_the_data_array() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"data":[{"model_group":"gpt-4o"}]}"#,
    )]);

    let (result, stdout) = run(OutputFormat::Json, &server);

    result.unwrap();
    assert_eq!(stdout, "[{\"model_group\":\"gpt-4o\"}]\n");
}

#[rstest]
#[case::an_object(r#"{"data":{}}"#, "object")]
#[case::a_string(r#"{"data":"text"}"#, "string")]
#[case::a_number(r#"{"data":7}"#, "number")]
#[case::a_bool(r#"{"data":true}"#, "bool")]
#[case::null(r#"{"data":null}"#, "null")]
fn list_rejects_data_that_is_not_a_list(#[case] body: &str, #[case] kind: &str) {
    let server = TestServer::start(vec![Answer::json(200, body)]);

    let (result, stdout) = run(OutputFormat::Table, &server);

    let Err(ModelGroupsError::UnexpectedType { kind: found }) = result else {
        panic!("expected an unexpected-type error");
    };
    assert_eq!(found, kind);
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn list_rejects_a_response_without_data() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, _stdout) = run(OutputFormat::Table, &server);

    assert!(matches!(result, Err(ModelGroupsError::Response { .. })));
}

#[test]
fn list_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);

    let (result, stdout) = run(OutputFormat::Table, &server);

    assert!(matches!(
        result,
        Err(ModelGroupsError::Http { status: 500 })
    ));
    assert!(stdout.is_empty(), "{stdout}");
}
