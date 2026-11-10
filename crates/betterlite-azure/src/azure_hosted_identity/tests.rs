#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::azure_hosted_identity::decode::decode_hosted_identity_token;
use crate::azure_hosted_identity::error::HostedIdentityError;
use crate::azure_hosted_identity::msal_reply::MsalKind;
use fixture::{Fixture, FixtureClock, NOW, Recorded, THUMBPRINT, only_request, response};

mod fixture;
mod scenarios;
mod selection;

#[rstest]
#[case::app_service_header(scenarios::app_service_header)]
#[case::cloud_shell_no_client_id(scenarios::cloud_shell_no_client_id)]
#[case::service_fabric_thumbprint(scenarios::service_fabric_thumbprint)]
#[case::service_fabric_client_id_rejected(scenarios::service_fabric_client_id_rejected)]
#[case::azure_ml_numeric_expiry(scenarios::azure_ml_numeric_expiry)]
#[case::cloud_shell_reply(scenarios::cloud_shell_reply)]
#[case::service_fabric_reply(scenarios::service_fabric_reply)]
#[case::hosted_errors(scenarios::hosted_errors)]
#[case::hosted_inner_selection(selection::hosted_inner_selection)]
fn azure_hosted_identity_cases(#[case] scenario: fn()) {
    scenario();
}

const SF_TOKEN: &str = r#"{"access_token":"sf-token","expires_on":1700003600,"token_type":"Bearer","resource":"https://cognitiveservices.azure.com"}"#;
const SHELL_TOKEN: &str = r#"{"access_token":"shell-token","expires_in":3600,"resource":"https://cognitiveservices.azure.com"}"#;

#[test]
fn azure_hosted_identity_credential_identity_isolation() {
    isolation_between_headers();
    isolation_between_client_ids();
    isolation_between_identity_configs();
}

#[rstest]
#[case::app_service_empty(MsalKind::AppService, &[])]
#[case::app_service_with_ml_settings(MsalKind::AppService, &[("MSI_ENDPOINT", "https://ml.test/msi"), ("MSI_SECRET", "secret")])]
#[case::azure_ml_with_app_service_settings(MsalKind::AzureMl, &[("IDENTITY_ENDPOINT", "https://app.test/msi"), ("IDENTITY_HEADER", "header")])]
#[case::service_fabric_without_header(MsalKind::ServiceFabric, &[("IDENTITY_ENDPOINT", "https://sf.test/msi")])]
fn hosted_protocols_need_their_own_environment_settings(
    #[case] kind: MsalKind,
    #[case] pairs: &[(&str, &str)],
) {
    let fixture = Fixture::new().with_environment(pairs);
    let (outcome, transport) =
        fixture.acquire(kind, vec![response(SF_TOKEN)], &FixtureClock::fixed(NOW));
    assert!(matches!(
        outcome,
        Err(HostedIdentityError::MissingEnvironment { .. })
    ));
    assert!(transport.recorded().is_empty());
}

#[test]
fn isolation_between_headers() {
    let clock = FixtureClock::fixed(NOW);
    let first = Fixture::new().with_environment(&[
        ("IDENTITY_ENDPOINT", "https://sf.test/first"),
        ("IDENTITY_HEADER", "first-header"),
        ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
    ]);
    let second = Fixture::new().with_environment(&[
        ("IDENTITY_ENDPOINT", "https://sf.test/second"),
        ("IDENTITY_HEADER", "second-header"),
        ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
    ]);
    let (first_outcome, first_transport) =
        first.acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    let (second_outcome, second_transport) =
        second.acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    first_outcome.unwrap();
    second_outcome.unwrap();
    let first_request = only_request(&first_transport);
    let second_request = only_request(&second_transport);
    assert_eq!(first_request.url, "https://sf.test/first");
    assert_eq!(second_request.url, "https://sf.test/second");
    assert_eq!(first_request.header("Secret"), Some("first-header"));
    assert_eq!(second_request.header("Secret"), Some("second-header"));
    assert_no_secret(&first_request, "second-header");
    assert_no_secret(&second_request, "first-header");
}

#[test]
fn isolation_between_client_ids() {
    let clock = FixtureClock::fixed(NOW);
    let first = Fixture::new()
        .with_environment(&[
            ("IDENTITY_ENDPOINT", "http://app.test/token"),
            ("IDENTITY_HEADER", "app-header"),
        ])
        .with_client_id("alpha");
    let second = Fixture::new()
        .with_environment(&[
            ("IDENTITY_ENDPOINT", "http://app.test/token"),
            ("IDENTITY_HEADER", "app-header"),
        ])
        .with_client_id("beta");
    let (first_outcome, first_transport) =
        first.acquire(MsalKind::AppService, vec![response(SF_TOKEN)], &clock);
    let (second_outcome, second_transport) =
        second.acquire(MsalKind::AppService, vec![response(SF_TOKEN)], &clock);
    first_outcome.unwrap();
    second_outcome.unwrap();
    let first_request = only_request(&first_transport);
    let second_request = only_request(&second_transport);
    assert_eq!(first_request.query_value("client_id"), Some("alpha"));
    assert_eq!(second_request.query_value("client_id"), Some("beta"));
    assert_no_secret(&first_request, "beta");
    assert_no_secret(&second_request, "alpha");
}

#[test]
fn isolation_between_identity_configs() {
    let clock = FixtureClock::fixed(NOW);
    let first = Fixture::new()
        .with_environment(&[("MSI_ENDPOINT", "http://shell.test/first")])
        .with_identity_config(&[("tenant", "alpha")]);
    let second = Fixture::new()
        .with_environment(&[("MSI_ENDPOINT", "http://shell.test/second")])
        .with_identity_config(&[("tenant", "beta")]);
    let (first_outcome, first_transport) =
        first.acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    let (second_outcome, second_transport) =
        second.acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    first_outcome.unwrap();
    second_outcome.unwrap();
    let first_request = only_request(&first_transport);
    let second_request = only_request(&second_transport);
    assert_eq!(first_request.url, "http://shell.test/first");
    assert_eq!(second_request.url, "http://shell.test/second");
    assert_eq!(first_request.form_value("tenant"), Some("alpha"));
    assert_eq!(second_request.form_value("tenant"), Some("beta"));
    assert_no_secret(&first_request, "beta");
    assert_no_secret(&second_request, "alpha");
}

fn assert_no_secret(request: &Recorded, secret: &str) {
    assert!(!request.url.contains(secret));
    for (_, value) in request.headers.iter().chain(request.form.iter()) {
        assert!(!value.contains(secret));
    }
}

#[test]
fn azure_hosted_identity_secret_safe_failure() {
    secret_safe_service_fabric();
    secret_safe_token();
    secret_safe_cloud_shell();
}

fn secret_safe_service_fabric() {
    let clock = FixtureClock::fixed(NOW);
    let secret = "fixture-identity-header-secret";
    let fixture = Fixture::new().with_environment(&[
        ("IDENTITY_ENDPOINT", "https://sf.test/token"),
        ("IDENTITY_HEADER", secret),
        ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
    ]);
    let body = format!(
        r#"{{"access_token":"fixture-access-token","error":{{"code":"SecretHeaderNotFound"}},"echo":"{secret}"}}"#
    );
    let (outcome, _) = fixture.acquire(MsalKind::ServiceFabric, vec![response(&body)], &clock);
    let error = outcome.unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(shown.contains("[redacted]"));
    assert!(shown.contains("REDACTED"));
    assert!(!shown.contains(secret));
    assert!(!shown.contains("fixture-access-token"));
    assert!(!format!("{:?}", fixture.hosted()).contains(secret));
}

#[test]
fn redactor_alone_misses_opaque_identity_values() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = r#"{"echo":"fixture-identity-header-secret"}"#;
    assert_eq!(redactor.redact_string(text), text);
}

fn secret_safe_token() {
    let clock = FixtureClock::fixed(NOW);
    let fixture = Fixture::new().with_environment(&[
        ("IDENTITY_ENDPOINT", "https://sf.test/token"),
        ("IDENTITY_HEADER", "fixture-identity-header-secret"),
        ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
    ]);
    let (outcome, _) = fixture.acquire(
        MsalKind::ServiceFabric,
        vec![response(
            r#"{"access_token":"fixture-access-token","expires_on":1700003600,"token_type":"Bearer"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    assert_eq!(token.token.expose_secret(), "fixture-access-token");
    assert!(!format!("{token:?}").contains("fixture-access-token"));
}

fn secret_safe_cloud_shell() {
    let clock = FixtureClock::fixed(NOW);
    let fixture = Fixture::new().with_environment(&[
        ("MSI_ENDPOINT", "http://shell.test/token"),
        ("MSI_SECRET", "fixture-msi-secret"),
    ]);
    let (outcome, _) = fixture.acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"fixture-access-token","echo":"fixture-msi-secret","note":"fixture-access-token","refresh_token":"fixture-refresh-value"}"#,
        )],
        &clock,
    );
    let error = outcome.unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("fixture-msi-secret"));
    assert!(!shown.contains("fixture-access-token"));
    assert!(!shown.contains("fixture-refresh-value"));
    assert!(shown.contains(r#""note":"****""#));
    assert!(shown.contains("[redacted]"));
}

#[test]
fn an_empty_identity_secret_leaves_the_failure_text_alone() {
    let fixture = Fixture::new().with_environment(&[
        ("MSI_ENDPOINT", "http://shell.test/token"),
        ("MSI_SECRET", ""),
    ]);
    let (outcome, _) = fixture.acquire_cloud_shell(
        vec![response(r#"{"expires_in":3600}"#)],
        &FixtureClock::fixed(NOW),
    );
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Unexpected response \"{\"expires_in\":3600}\""
    );
}

#[test]
fn replays_the_azure_hosted_identity_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_hosted_identity");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let prefix = name.split('_').next().unwrap();
            let category = match decode_hosted_identity_token(&bytes) {
                Ok(Some(_)) => "ok",
                Ok(None) => "empty",
                Err(HostedIdentityError::ReplyJson(_)) => "syntax",
                Err(HostedIdentityError::ReplyValue { .. }) => "value",
                Err(other) => panic!("unexpected decoder error for {name}: {other}"),
            };
            assert_eq!(category, prefix, "{name}");
            replayed = replayed.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed > 0_usize);
}
