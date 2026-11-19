#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use betterlite_config::sections::key_management_settings::Settings as KeySettings;
use betterlite_config::tls::VerifySetting;
use betterlite_secret_managers::custom_interface::CustomValue;
use betterlite_secret_managers::secret_read_dispatch::http_transport::TlsVerification;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;

use super::dispatch_params::{access_mode_name, dispatch_settings};
use super::{parse_system, select_secret_manager, vault_tls};
use crate::config_boot::error::ConfigBootError;
use crate::config_boot::test_support::capture_logs;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings() -> KeySettings {
    KeySettings {
        hosted_keys: None,
        store_virtual_keys: false,
        prefix_for_stored_virtual_keys: "litellm/".to_owned(),
        access_mode: SecretAccessMode::ReadOnly,
        primary_secret_name: None,
        description: None,
        tags: None,
        kms_key_id: None,
        custom_secret_manager: None,
        aws_region_name: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: Vec::new(),
    }
}

fn full_settings() -> KeySettings {
    let mut settings = settings();
    settings.hosted_keys = Some(vec!["virtual-key-1".to_owned()]);
    settings.store_virtual_keys = true;
    settings.prefix_for_stored_virtual_keys = "prefix/".to_owned();
    settings.access_mode = SecretAccessMode::ReadAndWrite;
    settings.primary_secret_name = Some("primary".to_owned());
    settings.description = Some("a description".to_owned());
    settings.tags = Some(BTreeMap::from([("team".to_owned(), "core".to_owned())]));
    settings.kms_key_id = Some("kms-id".to_owned());
    settings.custom_secret_manager = Some("my.manager.Custom".to_owned());
    settings.aws_region_name = Some("eu-west-1".to_owned());
    settings.aws_role_name = Some("role".to_owned());
    settings.aws_session_name = Some("session".to_owned());
    settings.aws_external_id = Some("external".to_owned());
    settings.aws_profile_name = Some("profile".to_owned());
    settings.aws_web_identity_token = Some(SecretString::from("token"));
    settings.aws_sts_endpoint = Some("https://sts.test".to_owned());
    settings.replica_regions = vec!["us-west-1".to_owned()];
    settings
}

fn source_text(error: &ConfigBootError) -> String {
    Error::source(error).unwrap().to_string()
}

#[test]
fn an_absent_system_selects_no_manager() {
    let state = select_secret_manager(None, &settings(), None, &env(&[]), None).unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert!(state.settings.is_none());
    assert!(state.system.is_none());
    assert_eq!(state.access_mode, SecretAccessMode::ReadOnly);
    assert!(state.hosted_keys.is_none());
}

#[rstest]
#[case("azure_key_vault", KeyManagementSystem::AzureKeyVault)]
#[case("google_kms", KeyManagementSystem::GoogleKms)]
#[case("aws_secret_manager", KeyManagementSystem::AwsSecretManager)]
#[case("aws_kms", KeyManagementSystem::AwsKms)]
#[case("google_secret_manager", KeyManagementSystem::GoogleSecretManager)]
#[case("hashicorp_vault", KeyManagementSystem::HashicorpVault)]
#[case("cyberark", KeyManagementSystem::Cyberark)]
#[case("custom", KeyManagementSystem::Custom)]
fn supported_system_names_parse(#[case] name: &str, #[case] expected: KeyManagementSystem) {
    assert_eq!(parse_system(name), Some(expected));
}

#[rstest]
#[case("local")]
#[case("")]
#[case("AzureKeyVault")]
#[case("AZURE_KEY_VAULT")]
#[case("hashicorp-vault")]
#[case("vault")]
fn other_system_names_do_not_parse(#[case] name: &str) {
    assert!(parse_system(name).is_none());
}

#[test]
fn the_local_system_is_refused() {
    let Err(error) = select_secret_manager(Some("local"), &settings(), None, &env(&[]), None)
    else {
        panic!("the local key management system was accepted");
    };
    assert_eq!(error.to_string(), "Invalid Key Management System selected");
    assert!(matches!(error, ConfigBootError::InvalidKeyManagementSystem));
}

#[test]
fn azure_without_a_uri_logs_the_install_hint() {
    let (logs, _guard) = capture_logs();
    let state =
        select_secret_manager(Some("azure_key_vault"), &settings(), None, &env(&[]), None).unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert!(state.settings.is_none());
    assert!(state.system.is_none());
    assert!(logs.contents().contains(
        "Error when loading keys from Azure Key Vault: Error when loading keys from Azure Key Vault: AZURE_KEY_VAULT_URI is not set. .Ensure you run `pip install azure-identity azure-keyvault-secrets`"
    ));
}

#[test]
fn azure_with_an_empty_uri_logs_the_install_hint() {
    let (logs, _guard) = capture_logs();
    let state = select_secret_manager(
        Some("azure_key_vault"),
        &settings(),
        None,
        &env(&[("AZURE_KEY_VAULT_URI", "")]),
        None,
    )
    .unwrap();
    assert!(state.system.is_none());
    assert!(
        logs.contents()
            .contains("AZURE_KEY_VAULT_URI is not set. .Ensure you run")
    );
}

#[test]
fn azure_with_a_uri_installs_the_client_without_logging() {
    let (logs, _guard) = capture_logs();
    let state = select_secret_manager(
        Some("azure_key_vault"),
        &settings(),
        None,
        &env(&[("AZURE_KEY_VAULT_URI", "https://vault.azure.net")]),
        None,
    )
    .unwrap();
    assert_eq!(state.system, Some(KeyManagementSystem::AzureKeyVault));
    assert!(state.settings.is_some());
    assert!(
        !logs
            .contents()
            .contains("Error when loading keys from Azure Key Vault")
    );
}

#[test]
fn vault_reaches_the_real_client() {
    let state = select_secret_manager(
        Some("hashicorp_vault"),
        &settings(),
        None,
        &env(&[("HCP_VAULT_TOKEN", "token")]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "HashicorpSecretManager");
    assert_eq!(state.system, Some(KeyManagementSystem::HashicorpVault));
}

#[test]
fn vault_without_credentials_is_refused() {
    let Err(error) =
        select_secret_manager(Some("hashicorp_vault"), &settings(), None, &env(&[]), None)
    else {
        panic!("a vault without credentials was accepted");
    };
    assert_eq!(
        error.to_string(),
        "initializing the Vault secret manager failed"
    );
    assert!(source_text(&error).contains("Missing Vault authentication credentials"));
}

#[test]
fn cyberark_reaches_the_real_client() {
    let state = select_secret_manager(
        Some("cyberark"),
        &settings(),
        None,
        &env(&[("CYBERARK_API_KEY", "key")]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "CyberArkSecretManager");
    assert_eq!(state.system, Some(KeyManagementSystem::Cyberark));
}

#[test]
fn cyberark_without_credentials_is_refused() {
    let Err(error) = select_secret_manager(Some("cyberark"), &settings(), None, &env(&[]), None)
    else {
        panic!("cyberark without credentials was accepted");
    };
    assert_eq!(
        error.to_string(),
        "initializing the CyberArk secret manager failed"
    );
    assert!(source_text(&error).contains("Missing CyberArk credentials"));
}

#[test]
fn aws_kms_without_a_region_is_refused() {
    let Err(error) = select_secret_manager(Some("aws_kms"), &settings(), None, &env(&[]), None)
    else {
        panic!("aws kms without a region was accepted");
    };
    assert_eq!(error.to_string(), "initializing the AWS KMS client failed");
    assert!(
        source_text(&error).contains("Missing required environment variable - AWS_REGION_NAME")
    );
}

#[test]
fn aws_kms_with_a_region_is_accepted() {
    let state = select_secret_manager(
        Some("aws_kms"),
        &settings(),
        None,
        &env(&[("AWS_REGION_NAME", "eu-west-1")]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert_eq!(state.system, Some(KeyManagementSystem::AwsKms));
}

#[test]
fn google_kms_without_credentials_is_refused() {
    let Err(error) = select_secret_manager(Some("google_kms"), &settings(), None, &env(&[]), None)
    else {
        panic!("google kms without credentials was accepted");
    };
    assert_eq!(
        error.to_string(),
        "initializing the Google KMS client failed"
    );
    assert!(
        source_text(&error)
            .contains("Missing required environment variable - GOOGLE_APPLICATION_CREDENTIALS")
    );
}

#[test]
fn google_kms_without_a_resource_name_is_refused() {
    let Err(error) = select_secret_manager(
        Some("google_kms"),
        &settings(),
        None,
        &env(&[("GOOGLE_APPLICATION_CREDENTIALS", "/creds.json")]),
        None,
    ) else {
        panic!("google kms without a resource name was accepted");
    };
    assert!(
        source_text(&error)
            .contains("Missing required environment variable - GOOGLE_KMS_RESOURCE_NAME")
    );
}

#[test]
fn google_kms_with_its_values_is_accepted() {
    let state = select_secret_manager(
        Some("google_kms"),
        &settings(),
        None,
        &env(&[
            ("GOOGLE_APPLICATION_CREDENTIALS", "/creds.json"),
            ("GOOGLE_KMS_RESOURCE_NAME", "projects/p/cryptoKeys/k"),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert_eq!(state.system, Some(KeyManagementSystem::GoogleKms));
}

#[test]
fn aws_secret_manager_warns_without_a_region() {
    let (logs, _guard) = capture_logs();
    let state = select_secret_manager(
        Some("aws_secret_manager"),
        &settings(),
        None,
        &env(&[]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert!(logs.contents().contains(
        "No AWS region found in environment. Ensure aws_region_name is set in key_management_settings or AWS_REGION_NAME/AWS_REGION/AWS_DEFAULT_REGION is set in environment."
    ));
}

#[test]
fn aws_secret_manager_accepts_any_region_variable() {
    let (logs, _guard) = capture_logs();
    select_secret_manager(
        Some("aws_secret_manager"),
        &settings(),
        None,
        &env(&[("AWS_DEFAULT_REGION", "us-east-1")]),
        None,
    )
    .unwrap();
    assert!(!logs.contents().contains("No AWS region found"));
}

#[test]
fn google_secret_manager_without_a_project_is_refused() {
    let Err(error) = select_secret_manager(
        Some("google_secret_manager"),
        &settings(),
        None,
        &env(&[]),
        None,
    ) else {
        panic!("google secret manager without a project was accepted");
    };
    assert_eq!(
        error.to_string(),
        "initializing the Google Secret Manager client failed"
    );
    assert!(source_text(&error).contains(
        "Google Secret Manager requires a project ID, please set 'GOOGLE_SECRET_MANAGER_PROJECT_ID' in your .env"
    ));
}

#[test]
fn google_secret_manager_with_a_project_is_accepted() {
    let state = select_secret_manager(
        Some("google_secret_manager"),
        &settings(),
        None,
        &env(&[("GOOGLE_SECRET_MANAGER_PROJECT_ID", "proj")]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert_eq!(state.system, Some(KeyManagementSystem::GoogleSecretManager));
}

#[test]
fn custom_without_a_config_path_is_refused() {
    let Err(error) = select_secret_manager(Some("custom"), &settings(), None, &env(&[]), None)
    else {
        panic!("a custom manager without a config path was accepted");
    };
    assert_eq!(
        error.to_string(),
        "CustomSecretManagerException - config_file_path is required to load custom secret manager"
    );
}

#[test]
fn custom_without_the_manager_field_is_refused() {
    let Err(error) = select_secret_manager(
        Some("custom"),
        &settings(),
        Some(Path::new("/config.yaml")),
        &env(&[]),
        None,
    ) else {
        panic!("a custom manager without its manager field was accepted");
    };
    assert_eq!(
        error.to_string(),
        "CustomSecretManagerException - custom_secret_manager field is required in key_management_settings"
    );
}

#[test]
fn custom_with_a_path_and_a_manager_is_accepted() {
    let mut settings = settings();
    settings.custom_secret_manager = Some("my.manager.Custom".to_owned());
    let state = select_secret_manager(
        Some("custom"),
        &settings,
        Some(Path::new("/config.yaml")),
        &env(&[]),
        None,
    )
    .unwrap();
    assert_eq!(state.client.class_name(), "NoneType");
    assert_eq!(state.system, Some(KeyManagementSystem::Custom));
}

#[test]
fn the_state_carries_the_key_management_settings() {
    let mut settings = settings();
    settings.primary_secret_name = Some("primary".to_owned());
    settings.access_mode = SecretAccessMode::WriteOnly;
    settings.hosted_keys = Some(vec!["key-1".to_owned()]);
    let state = select_secret_manager(
        Some("hashicorp_vault"),
        &settings,
        None,
        &env(&[("HCP_VAULT_TOKEN", "token")]),
        None,
    )
    .unwrap();
    assert_eq!(state.access_mode, SecretAccessMode::WriteOnly);
    assert_eq!(state.hosted_keys, Some(vec!["key-1".to_owned()]));
    let stored = state.settings.unwrap();
    assert_eq!(stored.primary_secret_name.as_deref(), Some("primary"));
    assert_eq!(stored.optional_params.len(), 17);
}

#[test]
fn the_store_settings_reach_the_dispatch_params() {
    let dispatch = dispatch_settings(&full_settings());
    assert_eq!(dispatch.primary_secret_name.as_deref(), Some("primary"));
    let params = dispatch.optional_params;
    assert_eq!(params.len(), 17);
    assert_eq!(
        params.get("hosted_keys"),
        Some(&CustomValue::Array(vec![CustomValue::String(
            "virtual-key-1".to_owned()
        )]))
    );
    assert_eq!(
        params.get("store_virtual_keys"),
        Some(&CustomValue::Bool(true))
    );
    assert_eq!(
        params.get("prefix_for_stored_virtual_keys"),
        Some(&CustomValue::String("prefix/".to_owned()))
    );
    assert_eq!(
        params.get("access_mode"),
        Some(&CustomValue::String("read_and_write".to_owned()))
    );
}

#[test]
fn the_text_settings_reach_the_dispatch_params() {
    let params = dispatch_settings(&full_settings()).optional_params;
    assert_eq!(
        params.get("primary_secret_name"),
        Some(&CustomValue::String("primary".to_owned()))
    );
    assert_eq!(
        params.get("description"),
        Some(&CustomValue::String("a description".to_owned()))
    );
    assert_eq!(
        params.get("tags"),
        Some(&CustomValue::Object(BTreeMap::from([(
            "team".to_owned(),
            CustomValue::String("core".to_owned())
        )])))
    );
    assert_eq!(
        params.get("kms_key_id"),
        Some(&CustomValue::String("kms-id".to_owned()))
    );
    assert_eq!(
        params.get("custom_secret_manager"),
        Some(&CustomValue::String("my.manager.Custom".to_owned()))
    );
}

#[test]
fn the_aws_settings_reach_the_dispatch_params() {
    let params = dispatch_settings(&full_settings()).optional_params;
    assert_eq!(
        params.get("aws_region_name"),
        Some(&CustomValue::String("eu-west-1".to_owned()))
    );
    assert_eq!(
        params.get("aws_role_name"),
        Some(&CustomValue::String("role".to_owned()))
    );
    assert_eq!(
        params.get("aws_session_name"),
        Some(&CustomValue::String("session".to_owned()))
    );
    assert_eq!(
        params.get("aws_external_id"),
        Some(&CustomValue::String("external".to_owned()))
    );
    assert_eq!(
        params.get("aws_profile_name"),
        Some(&CustomValue::String("profile".to_owned()))
    );
    assert_eq!(
        params.get("aws_web_identity_token"),
        Some(&CustomValue::String("token".to_owned()))
    );
    assert_eq!(
        params.get("aws_sts_endpoint"),
        Some(&CustomValue::String("https://sts.test".to_owned()))
    );
    assert_eq!(
        params.get("replica_regions"),
        Some(&CustomValue::Array(vec![CustomValue::String(
            "us-west-1".to_owned()
        )]))
    );
}

#[test]
fn absent_optional_values_become_null() {
    let params = dispatch_settings(&settings()).optional_params;
    assert_eq!(params.get("hosted_keys"), Some(&CustomValue::Null));
    assert_eq!(params.get("primary_secret_name"), Some(&CustomValue::Null));
    assert_eq!(params.get("description"), Some(&CustomValue::Null));
    assert_eq!(params.get("tags"), Some(&CustomValue::Null));
    assert_eq!(params.get("kms_key_id"), Some(&CustomValue::Null));
    assert_eq!(
        params.get("custom_secret_manager"),
        Some(&CustomValue::Null)
    );
    assert_eq!(
        params.get("aws_web_identity_token"),
        Some(&CustomValue::Null)
    );
    assert_eq!(
        params.get("store_virtual_keys"),
        Some(&CustomValue::Bool(false))
    );
    assert_eq!(
        params.get("prefix_for_stored_virtual_keys"),
        Some(&CustomValue::String("litellm/".to_owned()))
    );
    assert_eq!(
        params.get("access_mode"),
        Some(&CustomValue::String("read_only".to_owned()))
    );
    assert_eq!(
        params.get("replica_regions"),
        Some(&CustomValue::Array(Vec::new()))
    );
}

#[test]
fn each_access_mode_has_its_litellm_name() {
    assert_eq!(access_mode_name(SecretAccessMode::ReadOnly), "read_only");
    assert_eq!(access_mode_name(SecretAccessMode::WriteOnly), "write_only");
    assert_eq!(
        access_mode_name(SecretAccessMode::ReadAndWrite),
        "read_and_write"
    );
}

#[test]
fn a_missing_ssl_verify_enables_verification() {
    assert_eq!(vault_tls(&env(&[]), None), TlsVerification::Enabled);
}

#[test]
fn an_empty_ssl_verify_enables_verification() {
    assert_eq!(
        vault_tls(&env(&[("SSL_VERIFY", "")]), None),
        TlsVerification::Enabled
    );
}

#[rstest]
#[case("true", TlsVerification::Enabled)]
#[case(" TRUE ", TlsVerification::Enabled)]
#[case("false", TlsVerification::Disabled)]
#[case("False", TlsVerification::Disabled)]
fn ssl_verify_flags_choose_the_verification(
    #[case] value: &str,
    #[case] expected: TlsVerification,
) {
    assert_eq!(vault_tls(&env(&[("SSL_VERIFY", value)]), None), expected);
}

#[rstest]
#[case("1")]
#[case("0")]
#[case("yes")]
#[case("no")]
#[case("/etc/ca.pem")]
fn other_ssl_verify_values_become_the_ca_file(#[case] value: &str) {
    assert_eq!(
        vault_tls(&env(&[("SSL_VERIFY", value)]), None),
        TlsVerification::CaFile(PathBuf::from(value))
    );
}

#[test]
fn an_existing_ssl_cert_file_is_used_when_verification_is_on() {
    let cert = NamedTempFile::new().unwrap();
    let path = cert.path().to_str().unwrap().to_owned();
    assert_eq!(
        vault_tls(&env(&[("SSL_CERT_FILE", path.as_str())]), None),
        TlsVerification::CaFile(PathBuf::from(&path))
    );
    assert_eq!(
        vault_tls(
            &env(&[("SSL_VERIFY", "true"), ("SSL_CERT_FILE", path.as_str())]),
            None
        ),
        TlsVerification::CaFile(PathBuf::from(&path))
    );
}

#[test]
fn a_missing_ssl_cert_file_leaves_verification_enabled() {
    assert_eq!(
        vault_tls(
            &env(&[("SSL_CERT_FILE", "/nonexistent/b8590/ca.pem")]),
            None
        ),
        TlsVerification::Enabled
    );
}

#[test]
fn ssl_cert_file_is_ignored_when_verification_is_off() {
    let cert = NamedTempFile::new().unwrap();
    let path = cert.path().to_str().unwrap().to_owned();
    assert_eq!(
        vault_tls(
            &env(&[("SSL_VERIFY", "false"), ("SSL_CERT_FILE", path.as_str())]),
            None
        ),
        TlsVerification::Disabled
    );
}

#[test]
fn a_litellm_settings_false_disables_verification() {
    assert_eq!(
        vault_tls(&env(&[]), Some(&VerifySetting::Bool(false))),
        TlsVerification::Disabled
    );
}

#[test]
fn a_litellm_settings_true_keeps_verification_on() {
    assert_eq!(
        vault_tls(&env(&[]), Some(&VerifySetting::Bool(true))),
        TlsVerification::Enabled
    );
}

#[test]
fn a_litellm_settings_true_still_uses_the_cert_file() {
    let cert = NamedTempFile::new().unwrap();
    let path = cert.path().to_str().unwrap().to_owned();
    assert_eq!(
        vault_tls(
            &env(&[("SSL_CERT_FILE", path.as_str())]),
            Some(&VerifySetting::Bool(true))
        ),
        TlsVerification::CaFile(PathBuf::from(&path))
    );
}

#[test]
fn an_empty_ssl_verify_falls_through_to_the_litellm_setting() {
    assert_eq!(
        vault_tls(
            &env(&[("SSL_VERIFY", "")]),
            Some(&VerifySetting::Bool(false))
        ),
        TlsVerification::Disabled
    );
}

#[test]
fn ssl_verify_env_wins_over_the_litellm_setting() {
    assert_eq!(
        vault_tls(
            &env(&[("SSL_VERIFY", "true")]),
            Some(&VerifySetting::Bool(false))
        ),
        TlsVerification::Enabled
    );
    assert_eq!(
        vault_tls(
            &env(&[("SSL_VERIFY", "false")]),
            Some(&VerifySetting::Bool(true))
        ),
        TlsVerification::Disabled
    );
}

#[test]
fn a_litellm_settings_path_becomes_the_ca_file() {
    assert_eq!(
        vault_tls(
            &env(&[]),
            Some(&VerifySetting::Path("/etc/vault-ca.pem".to_owned()))
        ),
        TlsVerification::CaFile(PathBuf::from("/etc/vault-ca.pem"))
    );
}

#[test]
fn a_litellm_settings_path_wins_over_the_cert_file() {
    let cert = NamedTempFile::new().unwrap();
    let path = cert.path().to_str().unwrap().to_owned();
    assert_eq!(
        vault_tls(
            &env(&[("SSL_CERT_FILE", path.as_str())]),
            Some(&VerifySetting::Path("/etc/vault-ca.pem".to_owned()))
        ),
        TlsVerification::CaFile(PathBuf::from("/etc/vault-ca.pem"))
    );
}

#[rstest]
#[case("true", TlsVerification::Enabled)]
#[case(" FALSE ", TlsVerification::Disabled)]
fn boolean_text_in_the_litellm_setting_is_converted(
    #[case] value: &str,
    #[case] expected: TlsVerification,
) {
    assert_eq!(
        vault_tls(&env(&[]), Some(&VerifySetting::Path(value.to_owned()))),
        expected
    );
}
