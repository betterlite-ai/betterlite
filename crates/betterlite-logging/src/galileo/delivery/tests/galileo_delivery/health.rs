use rstest::rstest;

use super::super::super::health::{HealthState, HealthStatus, check};
use super::{API_KEY_ENV, ScriptedTransport, TOKEN_ENV, delivery, response, unreachable_host};

fn unhealthy(message: &str) -> HealthStatus {
    HealthStatus {
        status: HealthState::Unhealthy,
        error_message: Some(message.to_owned()),
    }
}

#[test]
fn a_missing_project_id_is_unhealthy_before_any_request() {
    let delivery = delivery(&[("GALILEO_API_KEY", "key")]);
    let transport = ScriptedTransport::default();
    assert_eq!(
        check(&delivery, &transport),
        unhealthy("GALILEO_PROJECT_ID environment variable not set")
    );
    assert!(transport.calls().is_empty());
}

#[test]
fn a_missing_base_url_is_unhealthy_before_any_request() {
    let delivery = delivery(&[
        ("GALILEO_USERNAME", "user"),
        ("GALILEO_PASSWORD", "pass"),
        ("GALILEO_PROJECT_ID", "project-1"),
    ]);
    let transport = ScriptedTransport::default();
    assert_eq!(
        check(&delivery, &transport),
        unhealthy("GALILEO_BASE_URL environment variable not set")
    );
    assert!(transport.calls().is_empty());
}

#[rstest]
#[case::nothing(&[])]
#[case::username_only(&[("GALILEO_USERNAME", "user")])]
#[case::password_only(&[("GALILEO_PASSWORD", "pass")])]
fn token_auth_needs_both_username_and_password(#[case] credentials: &[(&str, &str)]) {
    let mut variables = vec![
        ("GALILEO_PROJECT_ID", "project-1"),
        ("GALILEO_BASE_URL", "https://galileo.example"),
    ];
    variables.extend_from_slice(credentials);
    let transport = ScriptedTransport::default();
    assert_eq!(
        check(&delivery(&variables), &transport),
        unhealthy(
            "GALILEO_API_KEY or GALILEO_USERNAME and GALILEO_PASSWORD environment variables must be set"
        )
    );
    assert!(transport.calls().is_empty());
}

#[test]
fn a_failed_login_is_an_authentication_failure() {
    let transport = ScriptedTransport::replying(vec![Ok(response(401, "{}"))]);
    assert_eq!(
        check(&delivery(TOKEN_ENV), &transport),
        unhealthy("Galileo authentication failed")
    );
}

#[test]
fn a_healthy_probe_gets_current_user_with_the_api_key_header() {
    let transport = ScriptedTransport::replying(vec![Ok(response(200, "{}"))]);
    let status = check(&delivery(API_KEY_ENV), &transport);
    assert_eq!(
        status,
        HealthStatus {
            status: HealthState::Healthy,
            error_message: None
        }
    );
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "GET");
    assert_eq!(calls[0].url, "https://api.galileo.ai/current_user");
    assert!(calls[0].headers.contains(&(
        "Galileo-API-Key".to_owned(),
        "key-0123456789abcdef".to_owned()
    )));
}

#[test]
fn token_auth_logs_in_then_probes_current_user() {
    let transport = ScriptedTransport::replying(vec![
        Ok(response(200, r#"{"access_token": "tok-123"}"#)),
        Ok(response(200, "{}")),
    ]);
    let status = check(&delivery(TOKEN_ENV), &transport);
    assert_eq!(status.status, HealthState::Healthy);
    let urls: Vec<String> = transport.calls().into_iter().map(|call| call.url).collect();
    assert_eq!(
        urls,
        [
            "https://galileo.example/login",
            "https://galileo.example/current_user"
        ]
    );
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::server_error(500)]
fn an_error_status_is_unhealthy_and_names_the_status(#[case] status: u16) {
    let transport = ScriptedTransport::replying(vec![Ok(response(status, "{}"))]);
    assert_eq!(
        check(&delivery(API_KEY_ENV), &transport),
        unhealthy(&format!("Galileo API returned HTTP {status}"))
    );
}

#[rstest]
#[case::ok(200)]
#[case::redirect(399)]
fn a_status_below_400_is_healthy(#[case] status: u16) {
    let transport = ScriptedTransport::replying(vec![Ok(response(status, "{}"))]);
    assert_eq!(
        check(&delivery(API_KEY_ENV), &transport).status,
        HealthState::Healthy
    );
}

#[test]
fn an_unreachable_probe_is_unhealthy_with_the_failure() {
    let transport = ScriptedTransport::replying(vec![unreachable_host()]);
    let status = check(&delivery(API_KEY_ENV), &transport);
    assert_eq!(status.status, HealthState::Unhealthy);
    let message = status.error_message.unwrap();
    assert!(
        message.starts_with("Galileo health check failed: "),
        "{message}"
    );
    assert!(message.contains("connection refused"), "{message}");
}
