#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{DEFAULT_MINIMUM_KEY_LENGTH, Settings};

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load("")?;
    Ok(config.sections.auth_context_sources)
}

fn secret(secret: Option<&secrecy::SecretString>) -> Option<&str> {
    secret.map(secrecy::ExposeSecret::expose_secret)
}

#[test]
fn sso_env_absent_and_empty() {
    let absent = load(&[]).expect("the config loads without sso settings");
    assert_eq!(absent.microsoft_client_id, None);
    assert_eq!(absent.google_client_id, None);
    assert_eq!(absent.generic_client_id, None);
    assert_eq!(absent.saml_idp_metadata_url, None);
    assert_eq!(absent.saml_idp_metadata_xml, None);
    assert_eq!(secret(absent.google_client_secret.as_ref()), None);
    assert_eq!(absent.microsoft_tenant, None);
    assert_eq!(absent.generic_authorization_endpoint, None);
    assert_eq!(absent.generic_token_endpoint, None);
    assert_eq!(absent.generic_userinfo_endpoint, None);
    assert_eq!(absent.minimum_custom_key_length, DEFAULT_MINIMUM_KEY_LENGTH);
}

#[test]
fn sso_env_empty_strings_kept() {
    let empty = load(&[
        ("MICROSOFT_CLIENT_ID", ""),
        ("GOOGLE_CLIENT_ID", ""),
        ("GENERIC_CLIENT_ID", ""),
        ("SAML_IDP_METADATA_URL", ""),
        ("SAML_IDP_METADATA_XML", ""),
        ("GOOGLE_CLIENT_SECRET", ""),
        ("MICROSOFT_CLIENT_SECRET", ""),
        ("MICROSOFT_TENANT", ""),
        ("GENERIC_CLIENT_SECRET", ""),
        ("GENERIC_AUTHORIZATION_ENDPOINT", ""),
        ("GENERIC_TOKEN_ENDPOINT", ""),
        ("GENERIC_USERINFO_ENDPOINT", ""),
        ("MINIMUM_CUSTOM_KEY_LENGTH", ""),
    ])
    .expect("empty variables are set, not absent");
    assert_eq!(empty.microsoft_client_id.as_deref(), Some(""));
    assert_eq!(empty.google_client_id.as_deref(), Some(""));
    assert_eq!(empty.generic_client_id.as_deref(), Some(""));
    assert_eq!(empty.saml_idp_metadata_url.as_deref(), Some(""));
    assert_eq!(empty.saml_idp_metadata_xml.as_deref(), Some(""));
    assert_eq!(secret(empty.google_client_secret.as_ref()), Some(""));
    assert_eq!(secret(empty.microsoft_client_secret.as_ref()), Some(""));
    assert_eq!(empty.microsoft_tenant.as_deref(), Some(""));
    assert_eq!(secret(empty.generic_client_secret.as_ref()), Some(""));
    assert_eq!(empty.generic_authorization_endpoint.as_deref(), Some(""));
    assert_eq!(empty.generic_token_endpoint.as_deref(), Some(""));
    assert_eq!(empty.generic_userinfo_endpoint.as_deref(), Some(""));
    assert_eq!(empty.minimum_custom_key_length, DEFAULT_MINIMUM_KEY_LENGTH);
}

#[test]
fn google_env_pair() {
    let settings = load(&[
        ("GOOGLE_CLIENT_ID", "google-client"),
        ("GOOGLE_CLIENT_SECRET", "google-secret"),
    ])
    .expect("the google pair loads");
    assert_eq!(settings.google_client_id.as_deref(), Some("google-client"));
    assert_eq!(
        secret(settings.google_client_secret.as_ref()),
        Some("google-secret")
    );
}

#[test]
fn microsoft_env_triplet() {
    let settings = load(&[
        ("MICROSOFT_CLIENT_ID", "microsoft-client"),
        ("MICROSOFT_CLIENT_SECRET", "microsoft-secret"),
        ("MICROSOFT_TENANT", "contoso"),
    ])
    .expect("the microsoft triplet loads");
    assert_eq!(
        settings.microsoft_client_id.as_deref(),
        Some("microsoft-client")
    );
    assert_eq!(
        secret(settings.microsoft_client_secret.as_ref()),
        Some("microsoft-secret")
    );
    assert_eq!(settings.microsoft_tenant.as_deref(), Some("contoso"));
}

#[test]
fn generic_env_five_fields() {
    let settings = load(&[
        ("GENERIC_CLIENT_ID", "generic-client"),
        ("GENERIC_CLIENT_SECRET", "generic-secret"),
        (
            "GENERIC_AUTHORIZATION_ENDPOINT",
            "https://idp.example/authorize",
        ),
        ("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"),
        ("GENERIC_USERINFO_ENDPOINT", "https://idp.example/userinfo"),
    ])
    .expect("the generic provider loads");
    assert_eq!(
        settings.generic_client_id.as_deref(),
        Some("generic-client")
    );
    assert_eq!(
        secret(settings.generic_client_secret.as_ref()),
        Some("generic-secret")
    );
    assert_eq!(
        settings.generic_authorization_endpoint.as_deref(),
        Some("https://idp.example/authorize")
    );
    assert_eq!(
        settings.generic_token_endpoint.as_deref(),
        Some("https://idp.example/token")
    );
    assert_eq!(
        settings.generic_userinfo_endpoint.as_deref(),
        Some("https://idp.example/userinfo")
    );
}

#[rstest]
#[case::url(&[("SAML_IDP_METADATA_URL", "https://idp.example/metadata")], Some("https://idp.example/metadata"), None)]
#[case::xml(&[("SAML_IDP_METADATA_XML", "<EntityDescriptor/>")], None, Some("<EntityDescriptor/>"))]
#[case::both(
    &[
        ("SAML_IDP_METADATA_URL", "https://idp.example/metadata"),
        ("SAML_IDP_METADATA_XML", "<EntityDescriptor/>"),
    ],
    Some("https://idp.example/metadata"),
    Some("<EntityDescriptor/>")
)]
fn saml_metadata_sources(
    #[case] environment: &[(&str, &str)],
    #[case] url: Option<&str>,
    #[case] xml: Option<&str>,
) {
    let settings = load(environment).expect("the saml metadata loads");
    assert_eq!(settings.saml_idp_metadata_url.as_deref(), url);
    assert_eq!(settings.saml_idp_metadata_xml.as_deref(), xml);
}

#[rstest]
#[case::absent(&[], DEFAULT_MINIMUM_KEY_LENGTH)]
#[case::configured(&[("MINIMUM_CUSTOM_KEY_LENGTH", "32")], 32)]
#[case::empty_is_unset(&[("MINIMUM_CUSTOM_KEY_LENGTH", "")], DEFAULT_MINIMUM_KEY_LENGTH)]
#[case::surrounding_spaces_trimmed(&[("MINIMUM_CUSTOM_KEY_LENGTH", " 32 ")], 32)]
fn minimum_key_default_override(#[case] environment: &[(&str, &str)], #[case] expected: i64) {
    let settings = load(environment).expect("the minimum key length loads");
    assert_eq!(settings.minimum_custom_key_length, expected);
}

#[rstest]
#[case::text("abc")]
#[case::float("1.5")]
#[case::signs("--1")]
fn minimum_key_invalid_startup(#[case] value: &str) {
    let error =
        load(&[("MINIMUM_CUSTOM_KEY_LENGTH", value)]).expect_err("invalid text is a startup error");
    assert!(matches!(error, ConfigError::Environment { .. }));
    assert!(error.to_string().contains("MINIMUM_CUSTOM_KEY_LENGTH"));
    assert!(!error.to_string().contains(value));
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
fn minimum_key_nonpositive(#[case] value: &str, #[case] expected: i64) {
    let settings = load(&[("MINIMUM_CUSTOM_KEY_LENGTH", value)])
        .expect("zero and negative values are not clamped");
    assert_eq!(settings.minimum_custom_key_length, expected);
}
