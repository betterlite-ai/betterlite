#![cfg(test)]

use secrecy::ExposeSecret as _;

use crate::azure_client_secret::transport::HttpMethod;
use crate::azure_hosted_identity::error::{HostedIdentityError, SERVICE_FABRIC_ERROR_MESSAGE};
use crate::azure_hosted_identity::msal_reply::MsalKind;
use crate::azure_hosted_identity::protocol::HostedProtocol;

use super::fixture::{
    ArcFixture, Fixture, FixtureClock, NOW, OTHER_THUMBPRINT, RESOURCE, THUMBPRINT, assert_pairs,
    only_request, response, typed_response,
};

const APP_SERVICE_ENVIRONMENT: [(&str, &str); 2] = [
    ("IDENTITY_ENDPOINT", "http://app.test/token"),
    ("IDENTITY_HEADER", "header-secret"),
];
const SERVICE_FABRIC_ENVIRONMENT: [(&str, &str); 3] = [
    ("IDENTITY_ENDPOINT", "https://sf.test/token"),
    ("IDENTITY_HEADER", "header-secret"),
    ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
];
const AZURE_ML_ENVIRONMENT: [(&str, &str); 2] = [
    ("MSI_ENDPOINT", "http://ml.test/token"),
    ("MSI_SECRET", "ml-secret"),
];
const CLOUD_SHELL_ENVIRONMENT: [(&str, &str); 1] = [("MSI_ENDPOINT", "http://shell.test/token")];

const SF_TOKEN: &str = r#"{"access_token":"sf-token","expires_on":1700003600,"token_type":"Bearer","resource":"https://cognitiveservices.azure.com"}"#;
const SHELL_TOKEN: &str = r#"{"access_token":"shell-token","expires_in":3600,"resource":"https://cognitiveservices.azure.com"}"#;

pub(super) fn app_service_header() {
    app_service_token();
    app_service_client_id();
    app_service_empty_client_id();
    app_service_rejections();
    app_service_refresh();
    app_service_clock_drift();
}

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

fn app_service_empty_client_id() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .with_client_id("")
        .acquire(MsalKind::AppService, vec![response(SF_TOKEN)], &clock);
    outcome.unwrap();
    assert_eq!(only_request(&transport).query_value("client_id"), None);
}

fn app_service_rejections() {
    let clock = FixtureClock::fixed(NOW);
    let fixture = Fixture::new().with_environment(&APP_SERVICE_ENVIRONMENT);
    assert_app_service_rejection(
        &fixture,
        r#"{"statusCode":400,"message":"no identity"}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) 400, no identity",
        &clock,
    );
    assert_app_service_rejection(
        &fixture,
        r#"{"access_token":"app-token","expires_on":0}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) None, None",
        &clock,
    );
    assert_app_service_rejection(
        &fixture,
        r#"{"access_token":"","expires_on":1700003600}"#,
        "App Service managed identity configuration not found in environment. Token request error: (invalid_scope) None, None",
        &clock,
    );
}

fn assert_app_service_rejection(
    fixture: &Fixture,
    body: &str,
    expected: &str,
    clock: &FixtureClock,
) {
    let (outcome, transport) = fixture.acquire(MsalKind::AppService, vec![response(body)], clock);
    assert_eq!(outcome.unwrap_err().to_string(), expected);
    assert_eq!(transport.recorded().len(), 1);
}

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

fn cloud_shell_missing_endpoint() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new().acquire_cloud_shell(vec![response(SHELL_TOKEN)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Cloud Shell managed identity configuration not found in environment. "
    );
}

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

fn service_fabric_empty_client_id() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, transport) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .with_client_id("")
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    outcome.unwrap();
    assert_eq!(transport.recorded().len(), 1);
}

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

fn azure_ml_zero_expiry() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .acquire(
            MsalKind::AzureMl,
            vec![response(r#"{"access_token":"ml-token","expires_on":0}"#)],
            &clock,
        );
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Azure ML managed identity configuration not found in environment. Token request error: (invalid_scope) {\"access_token\":\"[redacted]\",\"expires_on\":0}"
    );
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

fn cloud_shell_empty_body() {
    let clock = FixtureClock::fixed(NOW);
    let fixture = Fixture::new().with_environment(&CLOUD_SHELL_ENVIRONMENT);
    let (empty, _) = fixture.acquire_cloud_shell(vec![response("")], &clock);
    assert_eq!(empty.unwrap_err().to_string(), "No token received.");
    let (mapping, _) = fixture.acquire_cloud_shell(vec![response("{}")], &clock);
    assert_eq!(mapping.unwrap_err().to_string(), "No token received.");
}

fn cloud_shell_missing_token() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&CLOUD_SHELL_ENVIRONMENT)
        .acquire_cloud_shell(vec![response(r#"{"access_token":"shell-token"}"#)], &clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "Unexpected response \"{\"access_token\":\"****\"}\""
    );
}

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
    service_fabric_error_codes();
}

fn service_fabric_token() {
    let clock = FixtureClock::fixed(NOW);
    let (outcome, _) = Fixture::new()
        .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
        .acquire(MsalKind::ServiceFabric, vec![response(SF_TOKEN)], &clock);
    let token = outcome.unwrap();
    assert_eq!(token.expires_on, 1_700_003_600);
    assert_eq!(token.refresh_on, None);
}

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

fn service_fabric_error_codes() {
    let clock = FixtureClock::fixed(NOW);
    let fixture = Fixture::new().with_environment(&SERVICE_FABRIC_ENVIRONMENT);
    assert_service_fabric_error(
        &fixture,
        r#"{"error":{"code":"SecretHeaderNotFound"}}"#,
        "unauthorized_client",
        &clock,
    );
    assert_service_fabric_error(
        &fixture,
        r#"{"error":{"code":"ManagedIdentityNotFound"}}"#,
        "invalid_client",
        &clock,
    );
    assert_service_fabric_error(
        &fixture,
        r#"{"error":{"code":"ArgumentNullOrEmpty"}}"#,
        "invalid_scope",
        &clock,
    );
    assert_service_fabric_error(
        &fixture,
        r#"{"error":{"code":"SomethingElse"}}"#,
        "invalid_request",
        &clock,
    );
    assert_service_fabric_error(&fixture, r#"{"error":{}}"#, "invalid_request", &clock);
}

fn assert_service_fabric_error(fixture: &Fixture, body: &str, mapped: &str, clock: &FixtureClock) {
    let (outcome, _) = fixture.acquire(MsalKind::ServiceFabric, vec![response(body)], clock);
    assert_eq!(
        outcome.unwrap_err().to_string(),
        format!(
            "Service Fabric managed identity configuration not found in environment. Token request error: ({mapped}) {body}"
        )
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

pub(super) fn hosted_inner_selection() {
    presence_selection();
    arc_selection();
}

fn presence_selection() {
    assert_eq!(
        ArcFixture::new()
            .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
            .select(),
        HostedProtocol::ServiceFabric
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&APP_SERVICE_ENVIRONMENT)
            .select(),
        HostedProtocol::AppService
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&AZURE_ML_ENVIRONMENT)
            .select(),
        HostedProtocol::AzureMl
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[
                ("IDENTITY_ENDPOINT", "http://arc.test/token"),
                ("IMDS_ENDPOINT", "http://arc.test/imds"),
            ])
            .select(),
        HostedProtocol::Arc
    );
    assert_eq!(ArcFixture::new().select(), HostedProtocol::Vm);
}

fn arc_selection() {
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[
                ("IDENTITY_ENDPOINT", ""),
                ("IDENTITY_HEADER", ""),
                ("IDENTITY_SERVER_THUMBPRINT", ""),
            ])
            .select(),
        HostedProtocol::ServiceFabric
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[("IDENTITY_ENDPOINT", ""), ("IDENTITY_HEADER", "")])
            .select(),
        HostedProtocol::AppService
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[("MSI_ENDPOINT", ""), ("MSI_SECRET", "")])
            .select(),
        HostedProtocol::AzureMl
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[
                ("IDENTITY_ENDPOINT", ""),
                ("IMDS_ENDPOINT", "http://arc.test/imds"),
            ])
            .select(),
        HostedProtocol::Vm
    );
    assert_eq!(
        ArcFixture::new()
            .with_file("/opt/azcmagent/bin/himds", "")
            .select(),
        HostedProtocol::Arc
    );
}
