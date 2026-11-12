#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::zeroize::Zeroize as _;

use super::aws_params::ExternalCredentialSource;
use super::external_account::ExternalAccountSource;
use super::{
    AuthorizedUserSource, ConfiguredCredentials, CredentialDecodeError, CredentialSource, KeyText,
    decode_vertex_credentials,
};

const SERVICE_ACCOUNT: &[u8] = br#"{"type":"service_account","project_id":"key-project","private_key_id":"key-id","private_key":"KEYMATERIAL","client_email":"sa@example.iam.gserviceaccount.com","universe_domain":"example.com"}"#;
const AUTHORIZED_USER: &[u8] = br#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","token_uri":"https://oauth.example/token","quota_project_id":"quota-project"}"#;
const EXTERNAL: &[u8] = br#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"file":"/run/token","format":{"type":"text"}},"service_account_impersonation_url":"https://iam.example/v1/projects/-/serviceAccounts/sa:generateAccessToken","universe_domain":"example.com"}"#;
const AWS_EXTERNAL: &[u8] = br#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:aws4_request","token_url":"https://sts.example/v1/token","credential_source":{"environment_id":"aws1","region_url":"https://metadata.example/region"},"aws_access_key_id":"AKIAEXAMPLE","aws_secret_access_key":"AWSSECRET","aws_region_name":"us-east-1","aws_external_id":"external"}"#;
const EXECUTABLE: &[u8] = br#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"executable":{"command":"gcloud","timeout_millis":1000}}}"#;

#[test]
fn decodes_service_account_credentials() {
    let decoded = decode_vertex_credentials(SERVICE_ACCOUNT).unwrap();
    let CredentialSource::ServiceAccount(account) = &decoded else {
        panic!("expected service account credentials");
    };
    assert_eq!(decoded.project_id(), Some("key-project"));
    assert_eq!(decoded.quota_project_id(), None);
    assert_eq!(account.client_email, "sa@example.iam.gserviceaccount.com");
    assert_eq!(account.private_key_id.as_deref(), Some("key-id"));
    assert_eq!(account.universe_domain.as_deref(), Some("example.com"));
    assert!(!format!("{decoded:?}").contains("KEYMATERIAL"));
}

#[test]
fn decodes_authorized_user_credentials() {
    let decoded = decode_vertex_credentials(AUTHORIZED_USER).unwrap();
    let CredentialSource::AuthorizedUser(user) = &decoded else {
        panic!("expected authorized user credentials");
    };
    assert_eq!(decoded.project_id(), None);
    assert_eq!(decoded.quota_project_id(), Some("quota-project"));
    assert_eq!(user.client_id, "client");
    assert_eq!(
        user.token_uri.as_deref(),
        Some("https://oauth.example/token")
    );
    let debug = format!("{decoded:?}");
    assert!(!debug.contains("CLIENTSECRET"));
    assert!(!debug.contains("REFRESHTOKEN"));
}

#[test]
fn decodes_external_account_credentials() {
    let decoded = decode_vertex_credentials(EXTERNAL).unwrap();
    let CredentialSource::ExternalAccount(account) = &decoded else {
        panic!("expected external account credentials");
    };
    assert_eq!(account.token_url, "https://sts.googleapis.com/v1/token");
    assert!(!account.aws.any());
    assert_eq!(account.aws_environment(), None);
    assert!(!account.is_executable());
    let ExternalCredentialSource::File {
        file,
        format: source_format,
    } = &account.credential_source
    else {
        panic!("expected a file credential source");
    };
    assert_eq!(file, "/run/token");
    assert_eq!(
        source_format
            .as_ref()
            .map(|format| format.format_type.as_str()),
        Some("text")
    );
    assert_eq!(
        decoded.project_id(),
        None,
        "external accounts recover no project"
    );
}

#[test]
fn decodes_explicit_aws_credentials() {
    let decoded = decode_vertex_credentials(AWS_EXTERNAL).unwrap();
    let CredentialSource::ExternalAccount(account) = &decoded else {
        panic!("expected external account credentials");
    };
    assert_eq!(account.aws_environment(), Some("aws1"));
    assert!(account.aws.any());
    assert_eq!(account.aws.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(account.token_url, "https://sts.example/v1/token");
    let debug = format!("{decoded:?}");
    assert!(!debug.contains("AWSSECRET"));
    assert!(!debug.contains("AKIAEXAMPLE"));
}

#[test]
fn refuses_executable_credentials() {
    let error = decode_vertex_credentials(EXECUTABLE).unwrap_err();
    assert!(matches!(error, CredentialDecodeError::Executable));
}

#[rstest]
#[case::non_utf8(&[0xff, 0xfe], "not UTF-8")]
#[case::not_json(b"not json", "not a credential document")]
#[case::json_array(b"[]", "not a credential document")]
#[case::missing_client_email(
    br#"{"type":"service_account","private_key":"KEY"}"#,
    "not a credential document"
)]
#[case::non_aws_environment_without_a_source(
    br#"{"type":"external_account","audience":"a","subject_token_type":"t","credential_source":{"environment_id":"azure1"}}"#,
    "not a credential document"
)]
fn rejects_undecodable_credentials(#[case] bytes: &[u8], #[case] message: &str) {
    let error = decode_vertex_credentials(bytes).unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
}

#[test]
fn bounds_credentials_bytes() {
    let oversized = vec![b' '; 0x10_0001];
    let error = decode_vertex_credentials(&oversized).unwrap_err();
    assert!(matches!(
        error,
        CredentialDecodeError::TooLarge { limit: 0x10_0000 }
    ));
    let boundary = vec![b' '; 0x10_0000];
    assert!(matches!(
        decode_vertex_credentials(&boundary).unwrap_err(),
        CredentialDecodeError::Json(_)
    ));
}

#[rstest]
fn replays_the_vertex_credentials_corpus(
    #[files("fuzz/corpus/vertex_credentials/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus seed is readable");
    let _outcome = decode_vertex_credentials(&bytes);
}

#[test]
fn a_service_account_document_carries_the_key_and_empty_absent_ids() {
    let decoded =
        decode_vertex_credentials(br#"{"private_key":"KEYMATERIAL","client_email":"sa@example"}"#)
            .unwrap();
    assert_eq!(
        decoded.document(),
        r#"{"type":"service_account","project_id":"","private_key_id":"","private_key":"KEYMATERIAL","client_email":"sa@example","universe_domain":null}"#
    );
}

#[test]
fn an_authorized_user_document_carries_the_client_and_refresh_secrets() {
    let decoded = decode_vertex_credentials(AUTHORIZED_USER).unwrap();
    assert_eq!(
        decoded.document(),
        r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","token_uri":"https://oauth.example/token","quota_project_id":"quota-project","universe_domain":null}"#
    );
}

#[test]
fn an_external_account_document_carries_its_source_and_aws_keys() {
    let document = decode_vertex_credentials(AWS_EXTERNAL).unwrap().document();
    assert_eq!(
        document,
        r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:aws4_request","token_url":"https://sts.example/v1/token","credential_source":{"environment_id":"aws1","region_url":"https://metadata.example/region"},"aws_access_key_id":"AKIAEXAMPLE","aws_secret_access_key":"AWSSECRET","aws_region_name":"us-east-1","aws_external_id":"external"}"#
    );
}

#[test]
fn credential_types_default_when_the_type_key_is_absent() {
    let user: AuthorizedUserSource =
        serde_json::from_str(r#"{"client_id":"c","client_secret":"s","refresh_token":"r"}"#)
            .unwrap();
    assert_eq!(user.credential_type, "authorized_user");
    let external: ExternalAccountSource = serde_json::from_str(
        r#"{"audience":"a","subject_token_type":"t","credential_source":{"file":"/run/token"}}"#,
    )
    .unwrap();
    assert_eq!(external.credential_type, "external_account");
}

#[rstest]
#[case::access_key_id("aws_access_key_id")]
#[case::secret_access_key("aws_secret_access_key")]
#[case::session_token("aws_session_token")]
#[case::region_name("aws_region_name")]
#[case::session_name("aws_session_name")]
#[case::profile_name("aws_profile_name")]
#[case::role_name("aws_role_name")]
#[case::web_identity_token("aws_web_identity_token")]
#[case::sts_endpoint("aws_sts_endpoint")]
#[case::external_id("aws_external_id")]
fn any_one_aws_key_marks_explicit_aws_credentials(#[case] key: &str) {
    let document = format!(
        r#"{{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:aws4_request","credential_source":{{"environment_id":"aws1"}},"{key}":"value"}}"#
    );
    let decoded = decode_vertex_credentials(document.as_bytes()).unwrap();
    let CredentialSource::ExternalAccount(account) = decoded else {
        panic!("expected external account credentials");
    };
    assert!(account.aws.any(), "{key}");
}

#[test]
fn credential_text_that_is_not_a_document_stays_out_of_debug() {
    let malformed = r#"{"type":"service_account","private_key":"KEYMATERIAL"#;
    let configured = ConfiguredCredentials::from_text(malformed.to_owned()).unwrap();
    assert_eq!(configured.as_text(), Some(malformed));
    assert!(!format!("{configured:?}").contains("KEYMATERIAL"));
}

#[test]
fn zeroizing_key_text_clears_the_key() {
    let mut key = KeyText("KEYMATERIAL".to_owned());
    key.zeroize();
    assert!(key.0.is_empty());
}

#[test]
fn a_non_aws_environment_id_falls_through_to_the_file_source() {
    let decoded = decode_vertex_credentials(
        br#"{"type":"external_account","audience":"a","subject_token_type":"t","credential_source":{"environment_id":"azure1","file":"/run/token"}}"#,
    )
    .unwrap();
    let CredentialSource::ExternalAccount(account) = &decoded else {
        panic!("expected external account credentials");
    };
    assert_eq!(account.aws_environment(), None);
    assert!(
        matches!(&account.credential_source, ExternalCredentialSource::File { file, .. } if file == "/run/token")
    );
    assert!(!decoded.document().contains("environment_id"));
}

#[test]
fn an_impersonated_document_is_the_file_text() {
    let text = r#"{"type":"impersonated_service_account","service_account_impersonation_url":"https://iam.example/sa:generateAccessToken","source_credentials":{"type":"authorized_user","client_id":"c","client_secret":"CLIENTSECRET","refresh_token":"r"},"quota_project_id":"quota-project"}"#;
    let decoded = decode_vertex_credentials(text.as_bytes()).unwrap();
    assert!(matches!(decoded, CredentialSource::Impersonated(_)));
    assert_eq!(decoded.document(), text);
    assert_eq!(decoded.project_id(), None);
    assert_eq!(decoded.quota_project_id(), None);
    assert!(!format!("{decoded:?}").contains("CLIENTSECRET"));
}

#[rstest]
#[case::workforce_user("external_account_authorized_user")]
#[case::gdch("gdch_service_account")]
fn refuses_credential_types_the_sdk_cannot_build(#[case] kind: &str) {
    let document =
        format!(r#"{{"type":"{kind}","private_key":"KEY","client_email":"sa@example"}}"#);
    let error = decode_vertex_credentials(document.as_bytes()).unwrap_err();
    assert!(
        matches!(&error, CredentialDecodeError::Unsupported { kind: found } if found == kind),
        "{error}"
    );
}
