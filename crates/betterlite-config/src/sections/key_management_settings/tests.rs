#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
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
    assert_eq!(settings.store_virtual_keys, Some(true));
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
    assert_eq!(
        settings.replica_regions,
        Some(vec![String::from("us-west-2")])
    );
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
    assert_eq!(settings.store_virtual_keys, Some(false));
    assert_eq!(settings.prefix_for_stored_virtual_keys, "litellm/");
}
