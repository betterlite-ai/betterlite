#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::azure_client_secret::transport::HttpMethod;
use crate::azure_hosted_identity::error::{HostedIdentityError, SERVICE_FABRIC_ERROR_MESSAGE};
use crate::azure_hosted_identity::msal_reply::MsalKind;

use super::fixture::{
    Fixture, FixtureClock, NOW, OTHER_THUMBPRINT, RESOURCE, THUMBPRINT, assert_pairs, only_request,
    response, typed_response,
};

pub(super) const APP_SERVICE_ENVIRONMENT: [(&str, &str); 2] = [
    ("IDENTITY_ENDPOINT", "http://app.test/token"),
    ("IDENTITY_HEADER", "header-secret"),
];
pub(super) const SERVICE_FABRIC_ENVIRONMENT: [(&str, &str); 3] = [
    ("IDENTITY_ENDPOINT", "https://sf.test/token"),
    ("IDENTITY_HEADER", "header-secret"),
    ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
];
pub(super) const AZURE_ML_ENVIRONMENT: [(&str, &str); 2] = [
    ("MSI_ENDPOINT", "http://ml.test/token"),
    ("MSI_SECRET", "ml-secret"),
];
const CLOUD_SHELL_ENVIRONMENT: [(&str, &str); 1] = [("MSI_ENDPOINT", "http://shell.test/token")];

pub(super) const SF_TOKEN: &str = r#"{"access_token":"sf-token","expires_on":1700003600,"token_type":"Bearer","resource":"https://cognitiveservices.azure.com"}"#;
const SHELL_TOKEN: &str = r#"{"access_token":"shell-token","expires_in":3600,"resource":"https://cognitiveservices.azure.com"}"#;

pub(super) fn app_service_header() {
    app_service_token();
    app_service_client_id();
    app_service_empty_client_id();
    app_service_rejections();
    app_service_refresh();
    app_service_clock_drift();
}

#[test]
fn app_service_token() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new().with_environment(&APP_SERVICE_ENVIRONMENT).acquire(
        MsalKind::AppService,
        vec![response(
            r#"{"access_token":"app-token","expires_on":"1700003600","token_type":"Custom","resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    let request = only_request(&transport);
    assert_eq!(request.method, HttpMethod::Get);
    assert_eq!(request.url, "http://app.test/token");
    assert_pairs(
        &request.query,
        &[("api-version", "2019-08-01"), ("resource", RESOURCE)],
    );
    assert_pairs(
        &request.headers,
        &[("X-IDENTITY-HEADER", "header-secret"), ("Metadata", "true")],
    );
    assert!(request.form.is_empty());
    assert_eq!(token.token.expose_secret(), "app-token");
    assert_eq!(token.expires_on, 1_700_003_600);
    assert_eq!(token.token_type, "Custom");
    assert_eq!(token.refresh_on, None);
}

#[test]
fn app_service_client_id() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .with_client_id("fixture-client")
        .acquire(MsalKind::AppService, vec![response(SF_TOKEN)], &clock);
    outcome.unwrap();
    assert_eq!(
        only_request(&transport).query_value("client_id"),
        Some("fixture-client")
    );
}

#[test]
fn app_service_empty_client_id() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .with_client_id("")
        .acquire(MsalKind::AppService, vec![response(SF_TOKEN)], &clock);
    outcome.unwrap();
    assert_eq!(only_request(&transport).query_value("client_id"), None);
}

#[test]
fn app_service_rejections() {
    let fixture = Fixture::new().with_environment(&APP_SERVICE_ENVIRONMENT);
    let kind = MsalKind::AppService;
    assert_rejection(
        &fixture,
        kind,
        r#"{"statusCode":400,"message":"no identity"}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) 400, no identity",
    );
    assert_rejection(
        &fixture,
        kind,
        r#"{"access_token":"app-token","expires_on":0}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) None, None",
    );
    assert_rejection(
        &fixture,
        kind,
        r#"{"access_token":"","expires_on":1700003600}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) None, None",
    );
}

fn assert_rejection(fixture: &Fixture, kind: MsalKind, body: &str, expected: &str) {
    let (outcome, transport) =
        fixture.acquire(kind, vec![response(body)], &FixtureClock::fixed(NOW));
    assert_eq!(outcome.unwrap_err().to_string(), expected);
    assert_eq!(transport.recorded().len(), 1);
}

#[test]
fn app_service_refresh() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .acquire(
            MsalKind::AppService,
            vec![response(
                r#"{"access_token":"app-token","expires_on":1700007200}"#,
            )],
            &clock,
        );
    assert_eq!(outcome.unwrap().refresh_on, Some(NOW + 3_600));
}

#[test]
fn app_service_clock_drift() {
    let clock = FixtureClock::scheduled(&[NOW, NOW + 2, NOW + 3], NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .acquire(
            MsalKind::AppService,
            vec![response(
                r#"{"access_token":"app-token","expires_on":1700003600}"#,
            )],
            &clock,
        );
    assert_eq!(outcome.unwrap().expires_on, 1_700_003_601);
}

pub(super) fn cloud_shell_no_client_id() {
    cloud_shell_client_id_guard();
    cloud_shell_identity_config_guard();
    cloud_shell_in_default_credential();
    cloud_shell_plain_request();
    cloud_shell_unrelated_config();
    cloud_shell_missing_endpoint();
    cloud_shell_empty_endpoint();
}

#[test]
fn cloud_shell_client_id_guard() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .with_client_id("fixture-client")
        .acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "client_id should not be set for cloud shell managed identity."
    );
    assert!(transport.recorded().is_empty());
}

#[test]
fn cloud_shell_identity_config_guard() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .with_identity_config(&[("object_id", "object")])
        .acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "identity_config must not contain the following keys: object_id, resource_id, client_id"
    );
    assert!(transport.recorded().is_empty());
}

#[test]
fn cloud_shell_in_default_credential() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .with_client_id("fixture-client")
        .within_default_credential()
        .acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    outcome.unwrap();
    let request = only_request(&transport);
    assert_eq!(request.method, HttpMethod::Post);
    assert_eq!(request.url, "http://shell.test/token");
    assert!(request.query.is_empty());
    assert_pairs(
        &request.form,
        &[("resource", RESOURCE), ("client_id", "fixture-client")],
    );
    assert_pairs(
        &request.headers,
        &[
            ("Metadata", "true"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
    );
}

#[test]
fn cloud_shell_plain_request() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    let token = outcome.unwrap();
    assert_eq!(token.token.expose_secret(), "shell-token");
    assert_eq!(token.expires_on, NOW + 3_600);
    assert_eq!(token.token_type, "Bearer");
    assert_eq!(token.refresh_on, None);
    assert_pairs(&only_request(&transport).form, &[("resource", RESOURCE)]);
}

#[test]
fn cloud_shell_unrelated_config() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .with_identity_config(&[("tenant", "contoso")])
        .acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    outcome.unwrap();
    assert_pairs(
        &only_request(&transport).form,
        &[("resource", RESOURCE), ("tenant", "contoso")],
    );
}

#[test]
fn cloud_shell_missing_endpoint() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Cloud Shell managed identity configuration not found in environment. "
    );
}

#[test]
fn cloud_shell_empty_endpoint() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&[("MSI_ENDPOINT", "")])
        .acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Cloud Shell managed identity configuration not found in environment. "
    );
}

pub(super) fn service_fabric_thumbprint() {
    service_fabric_request();
    service_fabric_ignores_thumbprint();
}

#[test]
fn service_fabric_request() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    let token = outcome.unwrap();
    let request = only_request(&transport);
    assert_eq!(request.url, "https://sf.test/token");
    assert_pairs(
        &request.query,
        &[
            ("api-version", "2019-07-01-preview"),
            ("resource", RESOURCE),
        ],
    );
    assert_pairs(&request.headers, &[("Secret", "header-secret")]);
    assert_eq!(token.expires_on, 1_700_003_600);
    assert_eq!(token.token_type, "Bearer");
}

#[test]
fn service_fabric_ignores_thumbprint() {
    let clock = FixtureClock::fixed(NOW);
    let (first_outcome, first_transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    first_outcome.unwrap();
    let first = only_request(&first_transport);
    let (second_outcome, second_transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .with_environment(&[("IDENTITY_SERVER_THUMBPRINT", OTHER_THUMBPRINT)])
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    second_outcome.unwrap();
    assert_eq!(only_request(&second_transport), first);
}

pub(super) fn service_fabric_client_id_rejected() {
    service_fabric_client_id_guard();
    service_fabric_identity_config_guard();
    service_fabric_empty_client_id();
    service_fabric_system_assigned();
}

#[test]
fn service_fabric_client_id_guard() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .with_client_id("fixture-client")
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        SERVICE_FABRIC_ERROR_MESSAGE
    );
    assert!(transport.recorded().is_empty());
}

#[test]
fn service_fabric_identity_config_guard() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .with_identity_config(&[("object_id", "object")])
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        SERVICE_FABRIC_ERROR_MESSAGE
    );
    assert!(transport.recorded().is_empty());
}

#[test]
fn service_fabric_empty_client_id() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .with_client_id("")
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    outcome.unwrap();
    assert_eq!(transport.recorded().len(), 1);
}

#[test]
fn service_fabric_system_assigned() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    outcome.unwrap();
    assert_eq!(
        only_request(&transport).header("Secret"),
        Some("header-secret")
    );
}

pub(super) fn azure_ml_numeric_expiry() {
    azure_ml_request();
    azure_ml_client_id();
    azure_ml_numeric_string();
    azure_ml_linux_date();
    azure_ml_windows_date();
    azure_ml_missing_fields();
    azure_ml_zero_expiry();
}

#[test]
fn azure_ml_request() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(
            MsalKind::AzureMl,
            vec![response(
                r#"{"access_token":"ml-token","expires_on":1700003600}"#,
            )],
            &clock,
        );
    let token = outcome.unwrap();
    let request = only_request(&transport);
    assert_eq!(request.url, "http://ml.test/token");
    assert_pairs(
        &request.query,
        &[("api-version", "2017-09-01"), ("resource", RESOURCE)],
    );
    assert_pairs(&request.headers, &[("secret", "ml-secret")]);
    assert_eq!(token.expires_on, 1_700_003_600);
    assert_eq!(token.token_type, "Bearer");
}

#[test]
fn azure_ml_client_id() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .with_client_id("fixture-client")
        .acquire(
            MsalKind::AzureMl,
            vec![response(
                r#"{"access_token":"ml-token","expires_on":1700003600}"#,
            )],
            &clock,
        );
    outcome.unwrap();
    let request = only_request(&transport);
    assert_pairs(
        &request.query,
        &[
            ("api-version", "2017-09-01"),
            ("resource", RESOURCE),
            ("clientid", "fixture-client"),
        ],
    );
    assert_eq!(request.query_value("client_id"), None);
}

#[test]
fn azure_ml_numeric_string() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(
            MsalKind::AzureMl,
            vec![response(
                r#"{"access_token":"ml-token","expires_on":"1700003600"}"#,
            )],
            &clock,
        );
    assert_eq!(outcome.unwrap().expires_on, 1_700_003_600);
}

#[test]
fn azure_ml_linux_date() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(
            MsalKind::AzureMl,
            vec![response(
                r#"{"access_token":"ml-token","expires_on":"06/20/2019 02:57:58 +00:00"}"#,
            )],
            &clock,
        );
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::ReplyValue { field } if field == "expires_on"
    ));
}

#[test]
fn azure_ml_windows_date() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(
            MsalKind::AzureMl,
            vec![response(
                r#"{"access_token":"ml-token","expires_on":"1/16/2020 5:24:12 AM +00:00"}"#,
            )],
            &clock,
        );
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::ReplyValue { field } if field == "expires_on"
    ));
}

#[test]
fn azure_ml_missing_fields() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(MsalKind::AzureMl, vec![response(r#"{"foo":1}"#)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Azure ML managed identity configuration not found in environment. Token request error: (invalid_scope) {\"foo\":1}"
    );
}

#[test]
fn azure_ml_zero_expiry() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(
            MsalKind::AzureMl,
            vec![response(r#"{"access_token":"ml-token","expires_on":0}"#)],
            &clock,
        );
    let message = outcome.unwrap_err().to_string();
    assert!(message.starts_with(
        "Azure ML managed identity configuration not found in environment. Token request error: (invalid_scope) "
    ));
    assert!(message.contains("REDACTED"));
    assert!(message.contains("\"expires_on\":0"));
    assert!(!message.contains("ml-token"));
}

pub(super) fn cloud_shell_reply() {
    cloud_shell_empty_body();
    cloud_shell_missing_token();
    cloud_shell_missing_expiry();
    cloud_shell_missing_resource();
    cloud_shell_typed_json_failure();
    cloud_shell_untyped_json_failure();
    cloud_shell_expires_on_wins();
    cloud_shell_zero_expires_on();
    cloud_shell_refresh_threshold();
    cloud_shell_refresh_just_above();
    cloud_shell_explicit_refresh();
    cloud_shell_explicit_refresh_above_threshold();
    cloud_shell_numeric_string();
}

#[test]
fn cloud_shell_empty_body() {
    let clock = FixtureClock::fixed(NOW);
    let fixture = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT);
    let (empty, _) = fixture.acquire_cloud_shell(vec![response("")], &clock);
    assert_eq!(empty.unwrap_err().to_string(), "No token received.");
    let (mapping, _) = fixture.acquire_cloud_shell(vec![response("{}")], &clock);
    assert_eq!(mapping.unwrap_err().to_string(), "No token received.");
}

#[test]
fn cloud_shell_missing_token() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![response(r#"{"access_token":"shell-token"}"#)], &clock);
    let message = outcome.unwrap_err().to_string();
    assert!(message.starts_with("Unexpected response \""));
    assert!(message.contains("REDACTED"));
    assert!(!message.contains("shell-token"));
}

#[test]
fn cloud_shell_missing_expiry() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![response(r#"{"expires_in":3600}"#)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Unexpected response \"{\"expires_in\":3600}\""
    );
}

#[test]
fn cloud_shell_unrecognized_keys_only() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![response(r#"{"foo":1}"#)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Unexpected response \"{\"foo\":1}\""
    );
}

#[test]
fn cloud_shell_missing_resource() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(
            vec![response(
                r#"{"access_token":"shell-token","expires_in":60}"#,
            )],
            &clock,
        );
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "the token reply is missing resource"
    );
}

#[test]
fn cloud_shell_typed_json_failure() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![typed_response("application/json", "not json")], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Failed to deserialize JSON from response"
    );
}

#[test]
fn cloud_shell_untyped_json_failure() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![typed_response("text/plain", "not json")], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Unexpected content type \"text/plain\""
    );
}

#[test]
fn cloud_shell_expires_on_wins() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_on":1700009999,"expires_in":100,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, 1_700_009_999);
    assert_eq!(token.refresh_on, None);
}

#[test]
fn cloud_shell_zero_expires_on() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_on":0,"expires_in":100,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    assert_eq!(outcome.unwrap().expires_on, NOW + 100);
}

#[test]
fn cloud_shell_refresh_threshold() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_in":7200,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, NOW + 7_200);
    assert_eq!(token.refresh_on, Some(NOW + 3_600));
}

#[test]
fn cloud_shell_no_refresh_below_threshold() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_in":7199,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, NOW + 7_199);
    assert_eq!(token.refresh_on, None);
}

#[test]
fn cloud_shell_refresh_just_above() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_in":7201,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    assert_eq!(outcome.unwrap().refresh_on, Some(NOW + 3_600));
}

#[test]
fn cloud_shell_explicit_refresh() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_in":3600,"refresh_in":600,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, NOW + 3_600);
    assert_eq!(token.refresh_on, Some(NOW + 600));
}

#[test]
fn cloud_shell_explicit_refresh_above_threshold() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_in":9000,"refresh_in":1200,"resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    assert_eq!(outcome.unwrap().refresh_on, Some(NOW + 1_200));
}

#[test]
fn cloud_shell_numeric_string() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT).acquire_cloud_shell(
        vec![response(
            r#"{"access_token":"shell-token","expires_in":"3600","token_type":"Custom","resource":"https://cognitiveservices.azure.com"}"#,
        )],
        &clock,
    );
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, NOW + 3_600);
    assert_eq!(token.token_type, "Custom");
}

pub(super) fn service_fabric_reply() {
    service_fabric_token();
    service_fabric_numeric_string();
    service_fabric_refresh_window();
    service_fabric_missing_token_type();
}

#[test]
fn service_fabric_token() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, 1_700_003_600);
    assert_eq!(token.refresh_on, None);
}

#[test]
fn service_fabric_numeric_string() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(
            MsalKind::ServiceFabric,
            vec![response(
                r#"{"access_token":"sf-token","expires_on":"1700003600","token_type":"Bearer"}"#,
            )],
            &clock,
        );
    assert_eq!(outcome.unwrap().expires_on, 1_700_003_600);
}

#[test]
fn service_fabric_refresh_window() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(
            MsalKind::ServiceFabric,
            vec![response(
                r#"{"access_token":"sf-token","expires_on":1700007200,"token_type":"Bearer"}"#,
            )],
            &clock,
        );
    assert_eq!(outcome.unwrap().refresh_on, Some(NOW + 3_600));
}

#[test]
fn service_fabric_missing_token_type() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(
            MsalKind::ServiceFabric,
            vec![response(
                r#"{"access_token":"sf-token","expires_on":1700003600}"#,
            )],
            &clock,
        );
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "the token reply is missing token_type"
    );
}

#[rstest]
#[case::secret_header(r#"{"error":{"code":"SecretHeaderNotFound"}}"#, "unauthorized_client")]
#[case::identity_not_found(r#"{"error":{"code":"ManagedIdentityNotFound"}}"#, "invalid_client")]
#[case::argument_empty(r#"{"error":{"code":"ArgumentNullOrEmpty"}}"#, "invalid_scope")]
#[case::other_code(r#"{"error":{"code":"SomethingElse"}}"#, "invalid_request")]
#[case::no_code(r#"{"error":{}}"#, "invalid_request")]
fn service_fabric_error_codes(#[case] body: &str, #[case] mapped: &str) {
    let fixture = Fixture::new().with_environment(&SERVICE_FABRIC_ENVIRONMENT);
    assert_rejection(
        &fixture,
        MsalKind::ServiceFabric,
        body,
        &format!(
            "Service Fabric managed identity configuration not found in environment. Token request error: ({mapped}) {body}"
        ),
    );
}

pub(super) fn hosted_errors() {
    assert_malformed_json(
        &Fixture::new().with_environment(&APP_SERVICE_ENVIRONMENT),
        MsalKind::AppService,
    );
    assert_malformed_json(
        &Fixture::new().with_environment(&AZURE_ML_ENVIRONMENT),
        MsalKind::AzureMl,
    );
    assert_malformed_json(
        &Fixture::new().with_environment(&SERVICE_FABRIC_ENVIRONMENT),
        MsalKind::ServiceFabric,
    );
    hosted_app_service_error_record();
    hosted_service_fabric_error_record();
    hosted_service_fabric_rejection();
    hosted_service_fabric_bad_error();
    hosted_string_error_member();
    hosted_cloud_shell_string_error_member();
    hosted_unread_members_are_ignored();
}

const OAUTH_ERROR: &str = r#"{"error":"invalid_request","error_description":"bad"}"#;

#[test]
fn hosted_string_error_member() {
    assert_rejection(
        &Fixture::new().with_environment(&APP_SERVICE_ENVIRONMENT),
        MsalKind::AppService,
        OAUTH_ERROR,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) None, None",
    );
    assert_rejection(
        &Fixture::new().with_environment(&AZURE_ML_ENVIRONMENT),
        MsalKind::AzureMl,
        OAUTH_ERROR,
        &format!(
            "Azure ML managed identity configuration not found in environment. Token request error: (invalid_scope) {OAUTH_ERROR}"
        ),
    );
}

#[test]
fn hosted_empty_identity_header_leaves_description_alone() {
    assert_rejection(
        &Fixture::new().with_environment(&[
            ("IDENTITY_ENDPOINT", "http://app.test/token"),
            ("IDENTITY_HEADER", ""),
        ]),
        MsalKind::AppService,
        r#"{"statusCode":400,"message":"no identity"}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) 400, no identity",
    );
}

#[test]
fn hosted_cloud_shell_string_error_member() {
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![response(OAUTH_ERROR)], &FixtureClock::fixed(NOW));
    assert_eq!(
        outcome.unwrap_err().to_string(),
        format!("Unexpected response \"{OAUTH_ERROR}\"")
    );
}

#[test]
fn hosted_unread_members_are_ignored() {
    assert_rejection(
        &Fixture::new().with_environment(&APP_SERVICE_ENVIRONMENT),
        MsalKind::AppService,
        r#"{"statusCode":{"a":1},"message":["x"],"expires_in":{"b":2},"refresh_in":[1],"error":7}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) <nested>, <nested>",
    );
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(
            vec![response(
                r#"{"access_token":"shell-token","expires_in":{"b":2},"resource":"r"}"#,
            )],
            &FixtureClock::fixed(NOW),
        );
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::ReplyValue { field } if field == "expires_in"
    ));
}

fn assert_malformed_json(fixture: &Fixture, protocol: MsalKind) {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = fixture.acquire(protocol, vec![response("not json")], &clock);
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::ReplyJson(_)
    ));
    assert_eq!(transport.recorded().len(), 1);
}

#[test]
fn hosted_app_service_error_record() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .acquire(
            MsalKind::AppService,
            vec![response(r#"{"statusCode":"500","message":"broken"}"#)],
            &clock,
        );
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::TokenRequest { .. }
    ));
}

#[test]
fn hosted_service_fabric_error_record() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(
            MsalKind::ServiceFabric,
            vec![response(r#"{"error":{"code":"ManagedIdentityNotFound"}}"#)],
            &clock,
        );
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::TokenRequest { .. }
    ));
}

#[test]
fn hosted_service_fabric_rejection() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .with_client_id("fixture-client")
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::ServiceFabricIdentityConfig
    ));
}

#[test]
fn hosted_service_fabric_bad_error() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(
            MsalKind::ServiceFabric,
            vec![response(r#"{"error":"not an object"}"#)],
            &clock,
        );
    assert!(matches!(
        outcome.unwrap_err(),
        HostedIdentityError::ReplyValue { .. }
    ));
}
