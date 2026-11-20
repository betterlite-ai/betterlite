#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

use super::{Raw, Settings};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn project(source: &str, lookup: &BTreeMap<String, String>) -> Settings {
    let (raw, _ignored) = parse_view::<Raw>(source).unwrap();
    raw.convert(lookup).unwrap()
}

#[test]
fn absent_general_settings_project_nothing() {
    let settings = project("{}", &env(&[]));
    assert!(settings.key_management_system.is_none());
    assert!(settings.key_management_settings.is_none());
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("{}")
    .unwrap();
    let loaded = &config.sections.secret_manager_bootstrap;
    assert!(loaded.key_management_system.is_none());
    assert!(loaded.key_management_settings.is_none());
}

#[test]
fn literal_manager_keys_load_through_the_config_loader() {
    let source = "general_settings:\n  key_management_system: hashicorp_vault\n  key_management_settings:\n    kms_key_id: os.environ/KMS_ID\n    hosted_keys: [virtual-key]\n    replica_regions: [os.environ/REGION]\n    tags:\n      Environment: os.environ/TAG\nmodel_list:\n- model_name: gpt\n  litellm_params:\n    model: openai/gpt-4o-mini\n    api_key: os.environ/OPENAI_API_KEY\n";
    let config = ConfigLoader {
        lookup: env(&[
            ("KMS_ID", "kms-id"),
            ("REGION", "us-east-1"),
            ("TAG", "Prod"),
            ("OPENAI_API_KEY", "sk-openai"),
        ]),
    }
    .load(source)
    .unwrap();
    let projection = &config.sections.secret_manager_bootstrap;
    assert_eq!(
        projection.key_management_system.as_deref(),
        Some("hashicorp_vault")
    );
    let settings = projection.key_management_settings.as_ref().unwrap();
    assert_eq!(settings["kms_key_id"], "os.environ/KMS_ID");
    assert_eq!(settings["replica_regions"][0], "os.environ/REGION");
    assert_eq!(settings["tags"]["Environment"], "os.environ/TAG");
    assert_eq!(
        config
            .sections
            .key_management_settings
            .kms_key_id
            .as_deref(),
        Some("kms-id")
    );
    assert_eq!(
        config.sections.key_management_settings.replica_regions,
        vec![String::from("us-east-1")]
    );
}

#[rstest]
#[case::environment_named("os.environ/KMS", "os.environ/KMS")]
#[case::empty("''", "")]
#[case::unsupported("bogus", "bogus")]
#[case::supported("hashicorp_vault", "hashicorp_vault")]
fn systems_are_projected_as_written(#[case] written: &str, #[case] expected: &str) {
    let source = format!("general_settings:\n  key_management_system: {written}\n");
    let settings = project(&source, &env(&[]));
    assert_eq!(settings.key_management_system.as_deref(), Some(expected));
}

#[test]
fn the_projection_accepts_a_config_whose_model_credential_is_absent() {
    let source = "general_settings:\n  key_management_system: hashicorp_vault\nmodel_list:\n- model_name: gpt\n  litellm_params:\n    model: openai/gpt-4o-mini\n    api_key: os.environ/MODEL_KEY\n";
    let settings = project(source, &env(&[]));
    assert_eq!(
        settings.key_management_system.as_deref(),
        Some("hashicorp_vault")
    );
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, .. } if variable == "MODEL_KEY")
    );
}
