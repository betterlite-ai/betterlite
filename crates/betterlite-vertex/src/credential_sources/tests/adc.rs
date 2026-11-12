#![cfg(test)]

use std::path::PathBuf;

use secrecy::ExposeSecret as _;

use super::fixtures::{FakeFiles, Fixture, TOKEN};
use crate::credential_sources::adc_access_token;
use crate::credential_sources::error::CredentialSourceError;

const APPLICATION_CREDENTIALS: &str = "/run/adc.json";
const GCLOUD_CREDENTIALS: &str = "/home/tester/.config/gcloud/application_default_credentials.json";
const GCLOUD_CONFIGURATION: &str = "/home/tester/.config/gcloud/configurations/config_default";
const SERVICE_ACCOUNT: &str = r#"{"type":"service_account","project_id":"key-project","private_key":"KEYMATERIAL","client_email":"sa@example.iam.gserviceaccount.com"}"#;
const AUTHORIZED_USER: &str = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN"}"#;

#[tokio::test]
async fn loads_application_default_credentials_from_the_environment_file() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().with(APPLICATION_CREDENTIALS, SERVICE_ACCOUNT.as_bytes());
    fixture.environment.application_credentials = Some(PathBuf::from(APPLICATION_CREDENTIALS));
    fixture.environment.project = Some("env-project".to_owned());
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "env-project");
    assert!(
        fixture.builders.calls()[0].starts_with("service_account "),
        "{}",
        fixture.builders.calls()[0]
    );
}

#[tokio::test]
async fn adc_recovers_the_service_account_project() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().with(APPLICATION_CREDENTIALS, SERVICE_ACCOUNT.as_bytes());
    fixture.environment.application_credentials = Some(PathBuf::from(APPLICATION_CREDENTIALS));
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "key-project");
}

#[tokio::test]
async fn a_missing_application_credentials_file_is_an_error() {
    let mut fixture = Fixture::new();
    fixture.environment.application_credentials = Some(PathBuf::from("/run/absent.json"));
    let error = fixture.load(None, None).await.unwrap_err();
    assert!(
        matches!(error, CredentialSourceError::Read { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn falls_back_to_the_gcloud_credentials_file() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().with(GCLOUD_CREDENTIALS, AUTHORIZED_USER.as_bytes());
    fixture.environment.gcloud_credentials = Some(PathBuf::from(GCLOUD_CREDENTIALS));
    fixture.environment.gcloud_project = Some("sdk-project".to_owned());
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "sdk-project");
    assert!(
        fixture.builders.calls()[0].starts_with("authorized_user "),
        "{}",
        fixture.builders.calls()[0]
    );
}

#[tokio::test]
async fn reads_the_gcloud_configuration_project() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default()
        .with(GCLOUD_CREDENTIALS, AUTHORIZED_USER.as_bytes())
        .with(GCLOUD_CONFIGURATION, b"core:\n  project: file-project\n");
    fixture.environment.gcloud_credentials = Some(PathBuf::from(GCLOUD_CREDENTIALS));
    fixture.environment.gcloud_configuration = Some(PathBuf::from(GCLOUD_CONFIGURATION));
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "file-project");
}

#[tokio::test]
async fn an_application_credentials_path_equal_to_the_gcloud_file_uses_the_gcloud_flow() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default()
        .with(GCLOUD_CREDENTIALS, AUTHORIZED_USER.as_bytes())
        .with(
            GCLOUD_CONFIGURATION,
            b"core:\n  project: configuration-project\n",
        );
    fixture.environment.application_credentials = Some(PathBuf::from(GCLOUD_CREDENTIALS));
    fixture.environment.gcloud_credentials = Some(PathBuf::from(GCLOUD_CREDENTIALS));
    fixture.environment.gcloud_configuration = Some(PathBuf::from(GCLOUD_CONFIGURATION));
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "configuration-project");
}

#[tokio::test]
async fn an_unreadable_gcloud_credentials_file_is_an_error() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().denying(GCLOUD_CREDENTIALS);
    fixture.environment.gcloud_credentials = Some(PathBuf::from(GCLOUD_CREDENTIALS));
    let error = fixture.load(None, None).await.unwrap_err();
    assert!(
        matches!(error, CredentialSourceError::Read { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn uses_the_metadata_server_without_credentials() {
    let mut fixture = Fixture::new();
    fixture.environment.metadata_host = Some("metadata.example:8080".to_owned());
    fixture.builders.metadata_project = Some("metadata-project".to_owned());
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "metadata-project");
    let calls = fixture.builders.calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0].starts_with(r#"metadata Some("metadata.example:8080")"#),
        "{}",
        calls[0]
    );
    assert!(
        calls[1].starts_with(r#"metadata_project Some("metadata.example:8080")"#),
        "{}",
        calls[1]
    );
}

#[tokio::test]
async fn a_configured_project_skips_the_metadata_project_lookup() {
    let mut fixture = Fixture::new();
    fixture.environment.metadata_host = Some("metadata.example:8080".to_owned());
    fixture.environment.project = Some("env-project".to_owned());
    let loaded = fixture.load(None, None).await.unwrap();
    assert_eq!(loaded.project, "env-project");
    assert_eq!(fixture.builders.calls().len(), 1);
}

#[tokio::test]
async fn adc_access_token_returns_the_refreshed_token() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().with(APPLICATION_CREDENTIALS, SERVICE_ACCOUNT.as_bytes());
    fixture.environment.application_credentials = Some(PathBuf::from(APPLICATION_CREDENTIALS));
    let token = adc_access_token("https://www.googleapis.com/auth/iam", &fixture.adc_deps())
        .await
        .unwrap();
    assert_eq!(token.expose_secret(), TOKEN);
    assert!(
        fixture.builders.calls()[0].contains("https://www.googleapis.com/auth/iam"),
        "{}",
        fixture.builders.calls()[0]
    );
}
