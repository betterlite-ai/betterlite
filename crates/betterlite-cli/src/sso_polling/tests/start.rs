use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::sso_polling::session::start_cli_sso_flow;

use super::fixtures::{PollCall, Reply, SERVER, response, scripted_transport, typed_response};

const STARTED: &str = r#"{"login_id":"login-1","poll_secret":"poll-1","user_code":"ABCD","verification_uri_complete":"https://gateway.example/verify"}"#;

#[test]
fn start_posts_to_the_start_path_and_returns_the_flow() {
    let transport = scripted_transport(vec![Reply::Response(response(200, STARTED))]);

    let flow = start_cli_sso_flow(SERVER, &transport).expect("the start flow");

    assert_eq!(flow.login_id, "login-1");
    assert_eq!(flow.poll_secret.expose_secret(), "poll-1");
    assert_eq!(flow.user_code, "ABCD");
    assert_eq!(
        flow.verification_uri_complete.as_deref(),
        Some("https://gateway.example/verify")
    );
    assert_eq!(
        transport.calls(),
        vec![PollCall::Post {
            url: format!("{SERVER}/sso/cli/start"),
            timeout: 10,
        }]
    );
}

#[test]
fn start_returns_a_flow_without_the_optional_uri() {
    let transport = scripted_transport(vec![Reply::Response(response(
        200,
        r#"{"login_id":"login-1","poll_secret":"poll-1","user_code":"ABCD"}"#,
    ))]);

    let flow = start_cli_sso_flow(SERVER, &transport).expect("the start flow");

    assert_eq!(flow.verification_uri_complete, None);
}

#[test]
fn start_accepts_an_empty_string_field_as_given() {
    // docs/compatibility.md: a field of the declared type is used as given, and LiteLLM only checks the type.
    let transport = scripted_transport(vec![Reply::Response(response(
        200,
        r#"{"login_id":"","poll_secret":"poll-1","user_code":"ABCD"}"#,
    ))]);

    let flow = start_cli_sso_flow(SERVER, &transport).expect("the start flow");

    assert!(flow.login_id.is_empty());
}

#[rstest]
#[case::not_found(404)]
#[case::method_not_allowed(405)]
fn start_explains_an_unsupported_endpoint(#[case] status: u16) {
    let transport = scripted_transport(vec![Reply::Response(response(status, ""))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the unsupported endpoint");

    assert!(
        error
            .to_string()
            .contains("does not support the CLI SSO login flow")
    );
}

#[test]
fn start_reports_a_server_status_with_its_detail() {
    let transport =
        scripted_transport(vec![Reply::Response(response(500, r#"{"detail":"boom"}"#))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the server status");

    let message = error.to_string();
    assert!(message.contains("HTTP 500"));
    assert!(message.contains("boom"));
}

#[test]
fn start_reports_a_server_status_without_a_detail() {
    let transport = scripted_transport(vec![Reply::Response(response(503, ""))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the server status");

    assert_eq!(
        error.to_string(),
        format!("starting CLI login failed: HTTP 503 from {SERVER}/sso/cli/start")
    );
}

#[test]
fn start_rejects_a_non_json_success_body() {
    let transport = scripted_transport(vec![Reply::Response(typed_response(
        200,
        "text/html",
        "<html></html>",
    ))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the html body");

    assert!(error.to_string().contains("content-type: text/html"));
}

#[test]
fn start_names_every_missing_field() {
    let transport = scripted_transport(vec![Reply::Response(response(200, "{}"))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the missing fields");

    assert!(
        error
            .to_string()
            .contains("login_id, poll_secret, user_code")
    );
}

#[test]
fn start_names_a_wrongly_typed_field() {
    let transport = scripted_transport(vec![Reply::Response(response(
        200,
        r#"{"login_id":5,"poll_secret":"poll-1","user_code":"ABCD"}"#,
    ))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the typed field");

    assert!(error.to_string().contains("login_id"));
}

#[test]
fn start_rejects_a_json_body_that_is_not_an_object() {
    let transport = scripted_transport(vec![Reply::Response(typed_response(
        200,
        "application/json",
        "[]",
    ))]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the array body");

    assert!(error.to_string().contains("not a valid CLI SSO start body"));
}

#[test]
fn start_reports_an_unreachable_proxy() {
    let transport = scripted_transport(vec![Reply::Transport]);

    let error = start_cli_sso_flow(SERVER, &transport).expect_err("the transport failure");

    assert!(error.to_string().contains("could not reach the proxy"));
}
