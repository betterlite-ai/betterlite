use std::fs;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::sap_auth::credentials::{fetch_credentials, validate_credentials};
use crate::sap_auth::error::SapAuthError;

use super::support::{FakeEnvironment, base_credentials, json};

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

#[test]
fn service_key_wins_over_every_other_source_without_merging() {
    let service_key = json(
        r#"{"credentials": {"clientid": "key-client"}, "clientsecret": "key-secret", "url": "https://key.example.com"}"#,
    );
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "env-client")
        .with_var("AICORE_BASE_URL", "https://env.example.com")
        .with_var("AICORE_RESOURCE_GROUP", "env-rg")
        .with_var(
            "VCAP_SERVICES",
            r#"{"aicore": [{"label": "aicore", "credentials": {"clientid": "vcap-client"}}]}"#,
        )
        .with_file(
            "/home/sap/.aicore/config.json",
            r#"{"client_id": "config-client", "base_url": "https://config.example.com"}"#,
        );
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("key-client"));
    assert_eq!(
        exposed(credentials.client_secret.as_ref()),
        Some("key-secret")
    );
    assert_eq!(
        credentials.auth_url.as_deref(),
        Some("https://key.example.com/oauth/token")
    );
    assert_eq!(credentials.base_url, None);
    assert_eq!(credentials.resource_group, "env-rg");
}

#[test]
fn service_key_is_read_from_the_environment_variable() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_SERVICE_KEY", r#"{"clientid": "env-service-key"}"#);
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("env-service-key"));
    assert_eq!(credentials.resource_group, "default");
}

#[test]
fn the_litellm_setting_is_used_before_the_environment_variable() {
    let setting = json(r#"{"clientid": "setting-client"}"#);
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_SERVICE_KEY", r#"{"clientid": "env-client"}"#);
    let credentials = fetch_credentials(&environment, None, Some(&setting)).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("setting-client"));
}

#[test]
fn an_empty_service_key_argument_falls_through() {
    let empty = json("\"\"");
    let setting = json(r#"{"clientid": "setting-client"}"#);
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&empty), Some(&setting)).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("setting-client"));
}

#[test]
fn invalid_json_in_a_service_key_is_skipped_for_later_sources() {
    let broken = json("\"not json\"");
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "env-client");
    let credentials = fetch_credentials(&environment, Some(&broken), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("env-client"));
}

#[test]
fn a_service_key_null_document_provides_nothing() {
    let null = json("null");
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "env-client");
    let credentials = fetch_credentials(&environment, Some(&null), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("env-client"));
}

#[test]
fn a_service_key_that_is_not_an_object_provides_nothing() {
    let number = json("7");
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let error = fetch_credentials(&environment, Some(&number), None).unwrap_err();
    assert!(matches!(error, SapAuthError::NoCredentials));
}

#[test]
fn environment_wins_over_the_config_file_and_vcap_without_merging() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "env-client")
        .with_var(
            "VCAP_SERVICES",
            r#"{"aicore": [{"label": "aicore", "credentials": {"clientid": "vcap-client"}}]}"#,
        )
        .with_file(
            "/home/sap/.aicore/config.json",
            r#"{"client_secret": "config-secret", "resource_group": "config-rg"}"#,
        );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("env-client"));
    assert!(credentials.client_secret.is_none());
    assert_eq!(credentials.resource_group, "config-rg");
}

#[test]
fn an_empty_environment_value_is_a_found_credential() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "")
        .with_file(
            "/home/sap/.aicore/config.json",
            r#"{"client_id": "config-client"}"#,
        );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some(""));
}

#[test]
fn the_resource_group_defaults_to_default() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "env-client");
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.resource_group, "default");
}

#[test]
fn an_empty_resource_group_is_used_as_given() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CLIENT_ID", "env-client")
        .with_var("AICORE_RESOURCE_GROUP", "");
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.resource_group, "");
}

#[rstest]
#[case::plain("https://auth.example.com", "https://auth.example.com/oauth/token")]
#[case::already_set(
    "https://auth.example.com/oauth/token",
    "https://auth.example.com/oauth/token"
)]
#[case::trailing_slash_before_suffix(
    "https://auth.example.com/oauth/token/",
    "https://auth.example.com/oauth/token/oauth/token"
)]
#[case::trailing_slashes("https://auth.example.com///", "https://auth.example.com/oauth/token")]
#[case::empty("", "/oauth/token")]
fn the_auth_url_gets_the_token_endpoint_suffix(#[case] input: &str, #[case] expected: &str) {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_AUTH_URL", input);
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.auth_url.as_deref(), Some(expected));
}

#[rstest]
#[case::plain("https://api.example.com", "https://api.example.com/v2")]
#[case::already_set("https://api.example.com/v2", "https://api.example.com/v2")]
#[case::trailing_slash("https://api.example.com/v2/", "https://api.example.com/v2/v2")]
#[case::trailing_slashes("https://api.example.com///", "https://api.example.com/v2")]
fn the_base_url_gets_the_v2_suffix(#[case] input: &str, #[case] expected: &str) {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_BASE_URL", input);
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.base_url.as_deref(), Some(expected));
}

#[test]
fn pem_strings_turn_escaped_newlines_into_newlines() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CERT_STR", "cert-line\\nmore-cert")
        .with_var("AICORE_KEY_STR", "key-line\\nmore-key");
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(
        credentials.cert_str.as_deref(),
        Some("cert-line\nmore-cert")
    );
    assert_eq!(
        exposed(credentials.key_str.as_ref()),
        Some("key-line\nmore-key")
    );
}

#[test]
fn a_cert_url_replaces_the_auth_url() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_AUTH_URL", "https://auth.example.com")
        .with_var("AICORE_CERT_URL", "https://cert.example.com");
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(
        credentials.auth_url.as_deref(),
        Some("https://cert.example.com/oauth/token")
    );
}

#[test]
fn the_credentials_path_is_tried_before_the_document_root() {
    let service_key = json(
        r#"{"credentials": {"clientid": "inner"}, "clientid": "outer", "clientsecret": "root-secret"}"#,
    );
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("inner"));
    assert_eq!(
        exposed(credentials.client_secret.as_ref()),
        Some("root-secret")
    );
}

#[test]
fn a_null_at_the_credentials_path_falls_back_to_the_document_root() {
    let service_key = json(r#"{"credentials": {"clientid": null}, "clientid": "root"}"#);
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("root"));
}

#[test]
fn certificate_paths_are_read_at_their_bare_name_in_a_service_key() {
    let service_key = json(
        r#"{"credentials": {"cert_file_path": "nested.crt", "key_file_path": "nested.key", "clientid": "c"}, "cert_file_path": "root.crt", "key_file_path": "root.key"}"#,
    );
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.cert_file_path.as_deref(), Some("root.crt"));
    assert_eq!(credentials.key_file_path.as_deref(), Some("root.key"));
}

#[test]
fn a_service_key_string_document_is_parsed_again() {
    let service_key = json(r#""{\"clientid\": \"double-encoded\"}""#);
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("double-encoded"));
}

#[test]
fn numeric_service_key_values_become_text() {
    let service_key = json(r#"{"clientid": 123}"#);
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("123"));
}

#[test]
fn list_service_key_values_are_skipped() {
    let service_key = json(r#"{"clientid": ["a"]}"#);
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let error = fetch_credentials(&environment, Some(&service_key), None).unwrap_err();
    assert!(matches!(error, SapAuthError::NoCredentials));
}

#[test]
fn the_service_key_resource_group_is_read_at_its_bare_name() {
    let service_key = json(
        r#"{"credentials": {"clientid": "c", "resource_group": "nested-rg"}, "resource_group": "bare-rg"}"#,
    );
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let credentials = fetch_credentials(&environment, Some(&service_key), None).unwrap();
    assert_eq!(credentials.resource_group, "bare-rg");
}

#[test]
fn vcap_picks_the_first_aicore_service_in_document_order() {
    let environment = FakeEnvironment::default().with_home("/home/sap").with_var(
        "VCAP_SERVICES",
        r#"{"z": [{"label": "other"}, {"label": "aicore", "credentials": {"clientid": "z"}}], "a": [{"label": "aicore", "credentials": {"clientid": "a"}}]}"#,
    );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("z"));
}

#[test]
fn vcap_reads_only_the_credentials_path_for_keyed_values() {
    let environment = FakeEnvironment::default().with_home("/home/sap").with_var(
        "VCAP_SERVICES",
        r#"{"aicore": [{"label": "aicore", "credentials": {"clientid": "v", "cert_file_path": "nested.crt"}, "cert_file_path": "bare.crt", "key_file_path": "bare.key", "resource_group": "vcap-rg"}]}"#,
    );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("v"));
    assert_eq!(credentials.cert_file_path.as_deref(), Some("bare.crt"));
    assert_eq!(credentials.key_file_path.as_deref(), Some("bare.key"));
    assert_eq!(credentials.resource_group, "vcap-rg");
}

#[rstest]
#[case::empty("")]
#[case::invalid_json("not json")]
#[case::json_null("null")]
#[case::json_list("[]")]
fn vcap_invalid_or_empty_json_means_no_service(#[case] raw: &str) {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("VCAP_SERVICES", raw);
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert!(matches!(error, SapAuthError::NoCredentials));
}

#[test]
fn vcap_entries_that_are_not_objects_are_skipped() {
    let environment = FakeEnvironment::default().with_home("/home/sap").with_var(
        "VCAP_SERVICES",
        r#"{"aicore": ["text", {"label": "aicore", "credentials": {"clientid": "v"}}]}"#,
    );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("v"));
}

#[test]
fn the_config_file_is_read_from_the_default_home() {
    let environment = FakeEnvironment::default().with_home("/home/sap").with_file(
        "/home/sap/.aicore/config.json",
        r#"{"client_id": "config-client"}"#,
    );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("config-client"));
}

#[test]
fn aicore_home_overrides_the_default_home() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_HOME", "/custom/home")
        .with_file(
            "/custom/home/config.json",
            r#"{"client_id": "custom-client"}"#,
        );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("custom-client"));
}

#[test]
fn a_profile_reads_the_profile_config_file() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_PROFILE", "dev")
        .with_file(
            "/home/sap/.aicore/config_dev.json",
            r#"{"client_id": "profile-client"}"#,
        );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("profile-client"));
}

#[rstest]
#[case::empty("")]
#[case::default("default")]
fn an_empty_or_default_profile_reads_config_json(#[case] profile: &str) {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_PROFILE", profile)
        .with_file(
            "/home/sap/.aicore/config.json",
            r#"{"client_id": "plain-client"}"#,
        );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("plain-client"));
}

#[test]
fn a_missing_default_config_file_provides_nothing() {
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert!(matches!(error, SapAuthError::NoCredentials));
}

#[test]
fn an_empty_config_path_falls_back_to_the_home_config_file() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CONFIG", "")
        .with_file(
            "/home/sap/.aicore/config.json",
            r#"{"client_id": "home-client"}"#,
        );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("home-client"));
}

#[test]
fn an_empty_profile_with_no_config_file_provides_nothing() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_PROFILE", "");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert!(matches!(error, SapAuthError::NoCredentials));
}

#[test]
fn an_explicit_config_path_that_is_missing_is_an_error() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_CONFIG", "/missing/config.json");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, SapAuthError::ConfigFileNotFound { .. }));
    assert_eq!(
        message,
        "Unable to locate profile config file at '/missing/config.json' in AICORE_HOME '/home/sap/.aicore'"
    );
}

#[test]
fn a_missing_profile_config_file_is_an_error() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_var("AICORE_PROFILE", "prod");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unable to locate profile config file at '/home/sap/.aicore/config_prod.json' in AICORE_HOME '/home/sap/.aicore'"
    );
}

#[test]
fn an_invalid_config_file_is_an_error_that_names_the_path() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .with_file("/home/sap/.aicore/config.json", "not json");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "/home/sap/.aicore/config.json is not valid JSON. Please fix or remove it!"
    );
}

#[test]
fn a_config_file_that_cannot_be_read_is_an_error() {
    let environment = FakeEnvironment::default()
        .with_home("/home/sap")
        .unreadable("/home/sap/.aicore/config.json");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert!(matches!(error, SapAuthError::ConfigRead { .. }));
}

#[test]
fn aicore_config_keys_win_and_null_falls_back_to_the_plain_name() {
    let environment = FakeEnvironment::default().with_home("/home/sap").with_file(
        "/home/sap/.aicore/config.json",
        r#"{"AICORE_CLIENT_ID": "upper", "client_id": "plain", "AICORE_CLIENT_SECRET": null, "client_secret": "plain-secret"}"#,
    );
    let credentials = fetch_credentials(&environment, None, None).unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some("upper"));
    assert_eq!(
        exposed(credentials.client_secret.as_ref()),
        Some("plain-secret")
    );
}

#[test]
fn a_config_file_that_is_not_an_object_provides_nothing() {
    let environment = FakeEnvironment::default().with_home("/home/sap").with_file(
        "/home/sap/.aicore/config.json",
        r#"[{"client_id": "list-client"}]"#,
    );
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert!(matches!(error, SapAuthError::NoCredentials));
}

#[test]
fn no_credentials_is_an_error() {
    let environment = FakeEnvironment::default().with_home("/home/sap");
    let error = fetch_credentials(&environment, None, None).unwrap_err();
    assert_eq!(error.to_string(), "No credentials found in any source");
}

#[test]
fn validation_accepts_a_client_secret() {
    let mut credentials = base_credentials();
    credentials.client_secret = Some(secret("secret"));
    validate_credentials(&credentials).unwrap();
}

#[test]
fn validation_accepts_a_pem_pair() {
    let mut credentials = base_credentials();
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret("key"));
    validate_credentials(&credentials).unwrap();
}

#[test]
fn validation_accepts_a_file_pair() {
    let mut credentials = base_credentials();
    credentials.cert_file_path = Some("cert.pem".to_owned());
    credentials.key_file_path = Some("key.pem".to_owned());
    validate_credentials(&credentials).unwrap();
}

#[test]
fn validation_reports_a_missing_auth_url() {
    let mut credentials = base_credentials();
    credentials.auth_url = None;
    let error = validate_credentials(&credentials).unwrap_err();
    assert_eq!(
        error.to_string(),
        "SAP AI Core credentials not found. Please provide credentials by setting appropriate environment variables (e.g. AICORE_CLIENT_ID, AICORE_CLIENT_SECRET, etc.)"
    );
}

#[test]
fn validation_reports_a_missing_client_id() {
    let mut credentials = base_credentials();
    credentials.client_id = None;
    let error = validate_credentials(&credentials).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("SAP AI Core credentials not found.")
    );
}

#[test]
fn validation_reports_a_missing_base_url() {
    let mut credentials = base_credentials();
    credentials.base_url = None;
    let error = validate_credentials(&credentials).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("SAP AI Core credentials not found.")
    );
}

#[test]
fn validation_rejects_an_empty_auth_url() {
    let mut credentials = base_credentials();
    credentials.auth_url = Some(String::new());
    let error = validate_credentials(&credentials).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("SAP AI Core credentials not found.")
    );
}

#[test]
fn validation_rejects_no_authentication_mode() {
    let credentials = base_credentials();
    let error = validate_credentials(&credentials).unwrap_err();
    assert_eq!(
        error.to_string(),
        "SAP AI Core credentials are incomplete. Invalid credentials: provide exactly one of client_secret, (cert_str & key_str), or (cert_file_path & key_file_path)."
    );
}

#[test]
fn validation_rejects_two_authentication_modes() {
    let mut credentials = base_credentials();
    credentials.client_secret = Some(secret("secret"));
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret("key"));
    let error = validate_credentials(&credentials).unwrap_err();
    assert_eq!(
        error.to_string(),
        "SAP AI Core credentials are incomplete. Invalid credentials: provide exactly one of client_secret, (cert_str & key_str), or (cert_file_path & key_file_path)."
    );
}

#[test]
fn validation_rejects_an_empty_client_secret() {
    let mut credentials = base_credentials();
    credentials.client_secret = Some(secret(""));
    let error = validate_credentials(&credentials).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("SAP AI Core credentials are incomplete.")
    );
}

#[test]
fn validation_rejects_a_pem_pair_with_an_empty_key() {
    let mut credentials = base_credentials();
    credentials.cert_str = Some("cert".to_owned());
    credentials.key_str = Some(secret(""));
    let error = validate_credentials(&credentials).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("SAP AI Core credentials are incomplete.")
    );
}

#[test]
fn validation_rejects_a_file_pair_with_one_path() {
    let mut credentials = base_credentials();
    credentials.cert_file_path = Some("cert.pem".to_owned());
    let error = validate_credentials(&credentials).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("SAP AI Core credentials are incomplete.")
    );
}

#[test]
fn replays_the_sap_credentials_corpus() {
    let seen = fs::read_dir("fuzz/corpus/sap_credentials")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .map(|path| fs::read(path).unwrap())
        .map(|bytes| {
            let Ok(text) = String::from_utf8(bytes) else {
                return;
            };
            let service_key = FakeEnvironment::default()
                .with_home("/fuzz")
                .with_var("AICORE_SERVICE_KEY", &text)
                .with_var("VCAP_SERVICES", &text);
            let _decoded = fetch_credentials(&service_key, None, None);
            let config = FakeEnvironment::default()
                .with_home("/fuzz")
                .with_var("AICORE_CONFIG", "/fuzz/config.json")
                .with_file("/fuzz/config.json", &text);
            let _configured = fetch_credentials(&config, None, None);
        })
        .count();
    assert!(seen > 0_usize);
}
