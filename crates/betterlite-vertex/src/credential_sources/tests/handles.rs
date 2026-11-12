#![cfg(test)]

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, decode_vertex_credentials,
};

use super::fixtures::{FakeFiles, Fixture, text};
use crate::credential_sources::error::CredentialSourceError;
use crate::credential_sources::handles::CLOUD_PLATFORM_SCOPE;

const SERVICE_ACCOUNT: &str = r#"{"type":"service_account","project_id":"key-project","private_key":"KEYMATERIAL","client_email":"sa@example.iam.gserviceaccount.com"}"#;
const SERVICE_ACCOUNT_WITHOUT_PROJECT: &str = r#"{"type":"service_account","private_key":"KEYMATERIAL","client_email":"sa@example.iam.gserviceaccount.com"}"#;
const AUTHORIZED_USER: &str = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","quota_project_id":"quota-project"}"#;
const AUTHORIZED_USER_WITHOUT_PROJECT: &str = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN"}"#;
const EXTERNAL_FILE: &str = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"file":"/run/token"}}"#;
const EXTERNAL_AWS: &str = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:aws4_request","token_url":"https://sts.example/v1/token","credential_source":{"environment_id":"aws1"},"aws_region_name":"us-east-1","aws_access_key_id":"AKIAEXAMPLE","aws_secret_access_key":"AWSSECRET"}"#;
const EXTERNAL_AWS_WITHOUT_KEYS: &str = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:aws4_request","credential_source":{"environment_id":"aws1"}}"#;
const EXTERNAL_AZURE: &str = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"environment_id":"azure1"}}"#;
const EXECUTABLE: &str = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"executable":{"command":"gcloud"}}}"#;

#[tokio::test]
async fn selects_the_service_account_constructor() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(Some(&text(SERVICE_ACCOUNT)), None)
        .await
        .unwrap();
    assert_eq!(loaded.project, "key-project");
    let calls = fixture.builders.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("service_account "), "{}", calls[0]);
    assert!(calls[0].contains(&format!("{CLOUD_PLATFORM_SCOPE:?}")));
    assert!(calls[0].contains(r#""project_id":"key-project""#));
    assert!(calls[0].contains(r#""private_key_id":"""#));
}

#[tokio::test]
async fn selects_the_authorized_user_constructor() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(Some(&text(AUTHORIZED_USER)), None)
        .await
        .unwrap();
    assert_eq!(loaded.project, "quota-project");
    assert!(
        fixture.builders.calls()[0].starts_with("authorized_user "),
        "{}",
        fixture.builders.calls()[0]
    );
    assert!(fixture.builders.calls()[0].contains(r#""quota_project_id":"quota-project""#));
}

#[tokio::test]
async fn selects_the_identity_pool_constructor() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(Some(&text(EXTERNAL_FILE)), Some("explicit-project"))
        .await
        .unwrap();
    assert_eq!(loaded.project, "explicit-project");
    let call = &fixture.builders.calls()[0];
    assert!(call.starts_with("identity_pool "), "{call}");
    assert!(
        call.contains(r#""token_url":"https://sts.googleapis.com/v1/token""#),
        "{call}"
    );
}

#[tokio::test]
async fn selects_the_aws_identity_pool_constructor() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(
            Some(&text(EXTERNAL_AWS_WITHOUT_KEYS)),
            Some("explicit-project"),
        )
        .await
        .unwrap();
    assert_eq!(loaded.project, "explicit-project");
    assert!(
        fixture.builders.calls()[0].starts_with("identity_pool_with_aws "),
        "{}",
        fixture.builders.calls()[0]
    );
}

#[tokio::test]
async fn selects_the_aws_federation_constructor() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(Some(&text(EXTERNAL_AWS)), Some("explicit-project"))
        .await
        .unwrap();
    assert_eq!(loaded.project, "explicit-project");
    let call = &fixture.builders.calls()[0];
    assert!(call.starts_with("aws_federation "), "{call}");
    assert!(
        call.contains(r#"Some("https://sts.example/v1/token")"#),
        "{call}"
    );
    assert!(call.contains(r#"Some("us-east-1")"#), "{call}");
}

#[tokio::test]
async fn refuses_an_identity_pool_environment_that_is_not_aws() {
    let fixture = Fixture::new();
    let error = fixture
        .load(Some(&text(EXTERNAL_AZURE)), Some("explicit-project"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CredentialSourceError::InvalidIdentityPoolEnvironment
    ));
}

#[tokio::test]
async fn resolves_a_project_for_a_key_that_has_none() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(
            Some(&text(SERVICE_ACCOUNT_WITHOUT_PROJECT)),
            Some("explicit-project"),
        )
        .await
        .unwrap();
    assert_eq!(loaded.project, "explicit-project");
    assert!(
        fixture.builders.calls()[0].contains(r#""project_id":"""#),
        "{}",
        fixture.builders.calls()[0]
    );
}

#[tokio::test]
async fn keeps_the_service_account_project_when_a_project_is_configured() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(Some(&text(SERVICE_ACCOUNT)), Some("explicit-project"))
        .await
        .unwrap();
    assert_eq!(loaded.project, "explicit-project");
    assert!(
        fixture.builders.calls()[0].contains(r#""project_id":"key-project""#),
        "{}",
        fixture.builders.calls()[0]
    );
}

#[tokio::test]
async fn refreshes_before_the_missing_project_error() {
    let mut fixture = Fixture::new();
    fixture.builders.fail_refresh = true;
    let error = fixture
        .load(Some(&text(AUTHORIZED_USER_WITHOUT_PROJECT)), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CredentialSourceError::Read { .. }));

    let healthy = Fixture::new();
    let missing = healthy
        .load(Some(&text(AUTHORIZED_USER_WITHOUT_PROJECT)), None)
        .await
        .unwrap_err();
    assert!(matches!(missing, CredentialSourceError::MissingProject));
}

#[tokio::test]
async fn an_empty_explicit_project_fails_after_the_refresh() {
    let fixture = Fixture::new();
    let error = fixture
        .load(Some(&text(SERVICE_ACCOUNT)), Some(""))
        .await
        .unwrap_err();
    assert!(matches!(error, CredentialSourceError::MissingProject));
    assert_eq!(fixture.builders.calls().len(), 1);
}

#[tokio::test]
async fn reads_credentials_from_an_existing_file() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().with("/run/vertex.json", SERVICE_ACCOUNT.as_bytes());
    let loaded = fixture
        .load(Some(&text("/run/vertex.json")), None)
        .await
        .unwrap();
    assert_eq!(loaded.project, "key-project");
    assert!(fixture.builders.calls()[0].starts_with("service_account "));
}

#[tokio::test]
async fn a_missing_credentials_file_is_a_parse_error() {
    let fixture = Fixture::new();
    let error = fixture
        .load(Some(&text("/run/absent.json")), None)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Unable to load vertex credentials from environment"),
        "{error}"
    );
}

#[tokio::test]
async fn refuses_an_executable_credentials_file() {
    let mut fixture = Fixture::new();
    fixture.files = FakeFiles::default().with("/run/executable.json", EXECUTABLE.as_bytes());
    let error = fixture
        .load(
            Some(&text("/run/executable.json")),
            Some("explicit-project"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("vertex_credentials"), "{error}");
    assert!(fixture.builders.calls().is_empty());
}

#[tokio::test]
async fn accepts_a_decoded_credential_source() {
    let fixture = Fixture::new();
    let source = decode_vertex_credentials(SERVICE_ACCOUNT.as_bytes()).unwrap();
    let loaded = fixture
        .load(Some(&ConfiguredCredentials::Source(source)), None)
        .await
        .unwrap();
    assert_eq!(loaded.project, "key-project");
}

#[tokio::test]
async fn errors_never_show_the_credential_material() {
    let fixture = Fixture::new();
    let document = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","token_uri":7}"#;
    let error = fixture.load(Some(&text(document)), None).await.unwrap_err();
    let debug = format!("{error:?}");
    assert!(!debug.contains("CLIENTSECRET"), "{debug}");
    assert!(!debug.contains("REFRESHTOKEN"), "{debug}");
    assert!(
        error
            .to_string()
            .contains("Unable to load vertex credentials from environment"),
        "{error}"
    );
}

#[tokio::test]
async fn the_loaded_auth_debug_shows_only_the_project() {
    let fixture = Fixture::new();
    let loaded = fixture
        .load(Some(&text(SERVICE_ACCOUNT)), None)
        .await
        .unwrap();
    let debug = format!("{loaded:?}");
    assert!(debug.contains("key-project"), "{debug}");
    assert!(!debug.contains("KEYMATERIAL"), "{debug}");
}
