#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};
use betterlite_secret_managers::secret_types::SecretAccessMode;

#[test]
fn config_secret_settings_reads_fields_and_defaults() {
    let config = ConfigLoader { lookup: BTreeMap::new() }
        .load("general_settings:\n  key_management_settings:\n    hosted_keys: [token]\n    access_mode: read_and_write\n    primary_secret_name: primary\n    description: managed\n    tags: {Environment: Prod}\n    store_virtual_keys: true\n    kms_key_id: key-1\n    aws_region_name: us-east-1\n    aws_role_name: role\n    aws_session_name: session\n    aws_external_id: external\n    aws_profile_name: profile\n    aws_web_identity_token: token-value\n    aws_sts_endpoint: https://sts.example\n    replica_regions: [us-west-2]\n")
        .expect("secret manager settings should load");
    let settings = config.sections.key_management_settings;
    assert_common_fields(&settings);
    assert_aws_fields(&settings);
}

fn assert_common_fields(settings: &super::Settings) {
    assert_eq!(settings.access_mode, SecretAccessMode::ReadAndWrite);
    assert_eq!(settings.hosted_keys, Some(vec![String::from("token")]));
    assert_eq!(settings.prefix_for_stored_virtual_keys, "litellm/");
    assert!(settings.store_virtual_keys);
    assert_eq!(settings.primary_secret_name.as_deref(), Some("primary"));
    assert_eq!(settings.description.as_deref(), Some("managed"));
    assert_eq!(
        settings.tags,
        Some(BTreeMap::from([(
            String::from("Environment"),
            String::from("Prod")
        )]))
    );
    assert_eq!(settings.kms_key_id.as_deref(), Some("key-1"));
}

fn assert_aws_fields(settings: &super::Settings) {
    assert!(!format!("{settings:?}").contains("token-value"));
    assert_eq!(settings.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(settings.aws_role_name.as_deref(), Some("role"));
    assert_eq!(settings.aws_session_name.as_deref(), Some("session"));
    assert_eq!(settings.aws_external_id.as_deref(), Some("external"));
    assert_eq!(settings.aws_profile_name.as_deref(), Some("profile"));
    assert_eq!(
        settings
            .aws_web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("token-value")
    );
    assert_eq!(settings.replica_regions, vec![String::from("us-west-2")]);
    assert_eq!(
        settings.aws_sts_endpoint.as_deref(),
        Some("https://sts.example")
    );
}

#[test]
fn config_secret_settings_resolves_environment_references() {
    let config = ConfigLoader {
        lookup: BTreeMap::from([
            (String::from("KMS_ID"), String::from("arn:aws:kms:region:key")),
            (String::from("WEB_TOKEN"), String::from("private-token")),
        ]),
    }
    .load("general_settings:\n  key_management_settings:\n    kms_key_id: os.environ/KMS_ID\n    aws_web_identity_token: os.environ/WEB_TOKEN\n")
    .expect("environment references should resolve");
    let settings = config.sections.key_management_settings;
    assert_eq!(
        settings.kms_key_id.as_deref(),
        Some("arn:aws:kms:region:key")
    );
    assert_eq!(
        settings
            .aws_web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("private-token")
    );
}

#[test]
fn custom_secret_manager_requires_a_class_path_when_custom_is_selected() {
    let config = ConfigLoader { lookup: BTreeMap::new() }
        .load("general_settings:\n  key_management_system: custom\n  key_management_settings:\n    custom_secret_manager: package.Manager\n")
        .expect("custom manager class path should be accepted");
    assert_eq!(
        config
            .sections
            .key_management_settings
            .custom_secret_manager
            .as_deref(),
        Some("package.Manager")
    );
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  key_management_system: custom\n")
    .expect_err("missing custom class path should fail");
    assert!(error.to_string().contains("custom_secret_manager"));
}

#[test]
fn config_secret_settings_has_litellm_defaults() {
    let settings = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("{}")
    .expect("empty config should load")
    .sections
    .key_management_settings;
    assert_eq!(settings.access_mode, SecretAccessMode::ReadOnly);
    assert!(!settings.store_virtual_keys);
    assert_eq!(settings.prefix_for_stored_virtual_keys, "litellm/");
    assert!(settings.hosted_keys.is_none());
    assert!(settings.tags.is_none());
    assert!(settings.replica_regions.is_empty());
}

#[test]
fn null_defaults_and_unknown_settings_are_accepted() {
    let settings = ConfigLoader { lookup: BTreeMap::new() }
        .load("general_settings:\n  key_management_settings:\n    access_mode: null\n    prefix_for_stored_virtual_keys: null\n    store_virtual_keys: null\n    replica_regions: null\n    future_option: {nested: true}\n")
        .unwrap().sections.key_management_settings;
    assert_eq!(settings.access_mode, SecretAccessMode::ReadOnly);
    assert_eq!(settings.prefix_for_stored_virtual_keys, "litellm/");
    assert!(!settings.store_virtual_keys);
    assert!(settings.replica_regions.is_empty());
}

#[rstest::rstest]
#[case::region("aws_region_name")]
#[case::role("aws_role_name")]
#[case::session("aws_session_name")]
#[case::external("aws_external_id")]
#[case::profile("aws_profile_name")]
#[case::endpoint("aws_sts_endpoint")]
#[case::token("aws_web_identity_token")]
#[case::prefix("prefix_for_stored_virtual_keys")]
#[case::primary("primary_secret_name")]
#[case::description("description")]
#[case::key("kms_key_id")]
#[case::custom("custom_secret_manager")]
fn missing_environment_names_the_field(#[case] field: &str) {
    let source =
        format!("general_settings:\n  key_management_settings:\n    {field}: os.environ/ABSENT\n");
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, path }
        if variable == "ABSENT" && path == &format!("general_settings.key_management_settings.{field}"))
    );
}

#[test]
fn injected_scalars_and_collections_resolve_without_process_environment() {
    let settings = ConfigLoader { lookup: BTreeMap::from([
        ("MODE".to_owned(), "write_only".to_owned()), ("FLAG".to_owned(), "true".to_owned()),
        ("NAME".to_owned(), "resolved".to_owned()),
    ]) }.load("general_settings:\n  key_management_settings:\n    access_mode: os.environ/MODE\n    store_virtual_keys: os.environ/FLAG\n    hosted_keys: [os.environ/NAME]\n    tags: {Environment: os.environ/NAME}\n    replica_regions: [os.environ/NAME]\n")
    .unwrap().sections.key_management_settings;
    assert_eq!(settings.access_mode, SecretAccessMode::WriteOnly);
    assert!(settings.store_virtual_keys);
    assert_eq!(settings.hosted_keys, Some(vec!["resolved".to_owned()]));
    assert_eq!(settings.replica_regions, vec!["resolved".to_owned()]);
    assert_eq!(
        settings.tags,
        Some(BTreeMap::from([(
            "Environment".to_owned(),
            "resolved".to_owned()
        )]))
    );
}

#[rstest::rstest]
#[case::access("access_mode: invalid")]
#[case::bool("store_virtual_keys: 'true'")]
#[case::list("hosted_keys: value")]
#[case::tags("tags: []")]
#[case::custom("custom_secret_manager: []")]
fn wrong_setting_types_and_values_are_rejected(#[case] setting: &str) {
    let source = format!("general_settings:\n  key_management_settings:\n    {setting}\n");
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap_err();
}

#[test]
fn explicit_empty_settings_and_optional_defaults_are_preserved() {
    let settings = ConfigLoader { lookup: BTreeMap::new() }
        .load("general_settings:\n  key_management_settings:\n    hosted_keys: []\n    prefix_for_stored_virtual_keys: ''\n    tags: {}\n    replica_regions: []\n")
        .unwrap().sections.key_management_settings;
    assert_eq!(settings.hosted_keys, Some(Vec::new()));
    assert_eq!(settings.prefix_for_stored_virtual_keys, "");
    assert_eq!(settings.tags, Some(BTreeMap::new()));
    assert!(settings.replica_regions.is_empty());
    assert!(settings.custom_secret_manager.is_none());
    assert!(settings.primary_secret_name.is_none());
    assert!(settings.description.is_none());
    assert!(settings.kms_key_id.is_none());
    assert_aws_defaults(&settings);
}

fn assert_aws_defaults(settings: &super::Settings) {
    assert!(settings.aws_region_name.is_none());
    assert!(settings.aws_role_name.is_none());
    assert!(settings.aws_session_name.is_none());
    assert!(settings.aws_external_id.is_none());
    assert!(settings.aws_profile_name.is_none());
    assert!(settings.aws_web_identity_token.is_none());
    assert!(settings.aws_sts_endpoint.is_none());
}

#[test]
fn empty_environment_values_use_defaults_and_redact_tokens() {
    let settings = ConfigLoader { lookup: BTreeMap::from([("EMPTY".to_owned(), String::new())]) }
        .load("general_settings:\n  key_management_settings:\n    access_mode: os.environ/EMPTY\n    store_virtual_keys: os.environ/EMPTY\n    prefix_for_stored_virtual_keys: os.environ/EMPTY\n    primary_secret_name: os.environ/EMPTY\n    aws_web_identity_token: os.environ/EMPTY\n")
        .unwrap().sections.key_management_settings;
    assert_eq!(settings.access_mode, SecretAccessMode::ReadOnly);
    assert!(!settings.store_virtual_keys);
    assert_eq!(settings.prefix_for_stored_virtual_keys, "litellm/");
    assert!(settings.primary_secret_name.is_none());
    assert!(settings.aws_web_identity_token.is_none());
}

#[test]
fn literal_empty_token_is_preserved() {
    let settings = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  key_management_settings:\n    aws_web_identity_token: ''\n")
    .unwrap()
    .sections
    .key_management_settings;
    assert_eq!(
        settings
            .aws_web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("")
    );
}
