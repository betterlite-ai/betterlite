#![cfg(test)]

use crate::commands::tables::OutputFormat;
use crate::commands::tests::support::{Answer, TestServer, client_for};

use super::{
    CreateCredential, CredentialsCommand, CredentialsError, DeleteCredential, GetCredential,
    ListCredentials, run_credentials_command,
};

fn run(
    command: CredentialsCommand,
    server: &TestServer,
) -> (Result<(), CredentialsError>, String, String) {
    let api = client_for(server, Some("sk-test"));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_credentials_command(command, &api, &mut stdout, &mut stderr);
    (
        result,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn list(format: OutputFormat) -> CredentialsCommand {
    CredentialsCommand::List(ListCredentials { format })
}

#[test]
fn list_table_prints_every_credential() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"credentials":[{"credential_name":"openai","credential_info":{"custom_llm_provider":"openai"}},{"credential_name":"bare"}]}"#,
    )]);

    let (result, stdout, stderr) = run(list(OutputFormat::Table), &server);

    result.unwrap();
    assert!(stdout.contains("Credentials"), "{stdout}");
    assert!(stdout.contains("Credential Name"), "{stdout}");
    assert!(stdout.contains("Custom LLM Provider"), "{stdout}");
    assert!(stdout.contains("openai"), "{stdout}");
    assert!(stdout.contains("bare"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /credentials HTTP/1.1");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
}

#[test]
fn list_json_prints_the_whole_response() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"credentials":[]}"#)]);

    let (result, stdout, stderr) = run(list(OutputFormat::Json), &server);

    result.unwrap();
    assert_eq!(stdout, "{\"credentials\":[]}\n");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn list_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);

    let (result, stdout, stderr) = run(list(OutputFormat::Table), &server);

    assert!(matches!(
        result,
        Err(CredentialsError::Http { status: 500 })
    ));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn list_rejects_a_response_that_is_not_an_object() {
    let server = TestServer::start(vec![Answer::json(200, "[]")]);

    let (result, stdout, _stderr) = run(list(OutputFormat::Table), &server);

    assert!(matches!(result, Err(CredentialsError::Response { .. })));
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn list_prints_an_empty_table_for_a_response_without_a_credentials_key() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"success":true}"#)]);

    let (result, stdout, stderr) = run(list(OutputFormat::Table), &server);

    result.unwrap();
    assert!(stdout.contains("Credentials"), "{stdout}");
    assert!(stdout.contains("Credential Name"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn get_prints_the_credential_response() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"credential_name":"openai"}"#)]);

    let (result, stdout, _stderr) = run(
        CredentialsCommand::Get(GetCredential {
            credential_name: String::from("openai"),
        }),
        &server,
    );

    result.unwrap();
    assert_eq!(stdout, "{\"credential_name\":\"openai\"}\n");
    let requests = server.finish();
    assert_eq!(
        requests[0].request_line,
        "GET /credentials/by_name/openai HTTP/1.1"
    );
}

#[test]
fn get_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(404, r#"{"detail":"missing"}"#)]);

    let (result, stdout, stderr) = run(
        CredentialsCommand::Get(GetCredential {
            credential_name: String::from("openai"),
        }),
        &server,
    );

    assert!(matches!(
        result,
        Err(CredentialsError::Http { status: 404 })
    ));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn create_posts_the_parsed_info_and_values() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"credential_name":"main"}"#)]);

    let (result, stdout, _stderr) = run(
        CredentialsCommand::Create(CreateCredential {
            credential_name: String::from("main"),
            info: String::from(r#"{"custom_llm_provider":"openai"}"#),
            values: String::from(r#"{"api_key":"sk-1"}"#),
        }),
        &server,
    );

    result.unwrap();
    assert_eq!(stdout, "{\"credential_name\":\"main\"}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /credentials HTTP/1.1");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(
        requests[0].body_text(),
        "{\"credential_name\":\"main\",\"credential_info\":{\"custom_llm_provider\":\"openai\"},\"credential_values\":{\"api_key\":\"sk-1\"}}"
    );
}

#[test]
fn create_rejects_invalid_json_as_a_usage_error_before_sending() {
    let server = TestServer::start(Vec::new());

    let (result, stdout, _stderr) = run(
        CredentialsCommand::Create(CreateCredential {
            credential_name: String::from("main"),
            info: String::from("not json"),
            values: String::from("{}"),
        }),
        &server,
    );

    let Err(error) = result else {
        panic!("expected an invalid-json error");
    };
    assert!(error.to_string().starts_with("Invalid JSON: "), "{error}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(server.finish().is_empty());
}

#[test]
fn create_prints_the_status_then_a_json_error_body_to_stdout() {
    let server = TestServer::start(vec![Answer::json(422, r#"{"detail":"bad"}"#)]);

    let (result, stdout, stderr) = run(
        CredentialsCommand::Create(CreateCredential {
            credential_name: String::from("main"),
            info: String::from("{}"),
            values: String::from("{}"),
        }),
        &server,
    );

    assert!(matches!(result, Err(CredentialsError::Aborted)));
    assert_eq!(stdout, "{\"detail\":\"bad\"}\n");
    assert_eq!(stderr, "Error: HTTP 422\n");
}

#[test]
fn create_prints_a_non_json_error_body_to_stderr() {
    let server = TestServer::start(vec![Answer::text(500, "the proxy broke")]);

    let (result, stdout, stderr) = run(
        CredentialsCommand::Create(CreateCredential {
            credential_name: String::from("main"),
            info: String::from("{}"),
            values: String::from("{}"),
        }),
        &server,
    );

    assert!(matches!(result, Err(CredentialsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(stderr, "Error: HTTP 500\nthe proxy broke\n");
}

#[test]
fn delete_sends_a_delete_for_the_named_credential() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"deleted":true}"#)]);

    let (result, stdout, _stderr) = run(
        CredentialsCommand::Delete(DeleteCredential {
            credential_name: String::from("old"),
        }),
        &server,
    );

    result.unwrap();
    assert_eq!(stdout, "{\"deleted\":true}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "DELETE /credentials/old HTTP/1.1");
}

#[test]
fn delete_prints_the_status_then_the_error_body() {
    let server = TestServer::start(vec![Answer::json(404, r#"{"detail":"missing"}"#)]);

    let (result, stdout, stderr) = run(
        CredentialsCommand::Delete(DeleteCredential {
            credential_name: String::from("old"),
        }),
        &server,
    );

    assert!(matches!(result, Err(CredentialsError::Aborted)));
    assert_eq!(stdout, "{\"detail\":\"missing\"}\n");
    assert_eq!(stderr, "Error: HTTP 404\n");
}
