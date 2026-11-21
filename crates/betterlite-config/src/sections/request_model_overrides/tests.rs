#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

fn load(source: &str, env: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .request_model_overrides)
}

#[derive(Debug)]
enum Expected {
    Loaded(Settings),
    InvalidScalar(&'static str),
    InvalidEnvironment,
}

#[rstest]
#[case::credential_overrides_enabled(
    "litellm_settings:\n  enable_model_config_credential_overrides: true\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: true,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::credential_overrides_disabled(
    "litellm_settings:\n  enable_model_config_credential_overrides: false\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::credential_overrides_unset(
    "litellm_settings:\n  drop_params: false\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::credential_overrides_null(
    "litellm_settings:\n  enable_model_config_credential_overrides: null\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::credential_overrides_text(
    "litellm_settings:\n  enable_model_config_credential_overrides: \"true\"\n",
    &[],
    Expected::InvalidScalar("litellm_settings.enable_model_config_credential_overrides")
)]
#[case::credential_overrides_number(
    "litellm_settings:\n  enable_model_config_credential_overrides: 1\n",
    &[],
    Expected::InvalidScalar("litellm_settings.enable_model_config_credential_overrides")
)]
#[case::enforced_params_list(
    "general_settings:\n  enforced_params: [user, metadata.generation_name]\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: Some(vec!["user".to_owned(), "metadata.generation_name".to_owned()]),
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::enforced_params_empty_list(
    "general_settings:\n  enforced_params: []\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: Some(Vec::new()),
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::enforced_params_null(
    "general_settings:\n  enforced_params: null\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::enforced_params_text(
    "general_settings:\n  enforced_params: user\n",
    &[],
    Expected::InvalidScalar("general_settings.enforced_params")
)]
#[case::enforced_params_non_text_item(
    "general_settings:\n  enforced_params: [user, 5]\n",
    &[],
    Expected::InvalidScalar("general_settings.enforced_params")
)]
#[case::service_account_params_list(
    "general_settings:\n  service_account_settings:\n    enforced_params: [user]\n",
    &[],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: Some(vec!["user".to_owned()]),
        team_stale_alias_bypass: false,
    })
)]
#[case::service_account_params_text(
    "general_settings:\n  service_account_settings:\n    enforced_params: user\n",
    &[],
    Expected::InvalidScalar("general_settings.service_account_settings.enforced_params")
)]
#[case::service_account_settings_not_object(
    "general_settings:\n  service_account_settings: [user]\n",
    &[],
    Expected::InvalidScalar("general_settings.service_account_settings")
)]
#[case::stale_alias_bypass_env_true(
    "",
    &[("LITELLM_ENABLE_TEAM_STALE_ALIAS_BYPASS", "true")],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: true,
    })
)]
#[case::stale_alias_bypass_env_off(
    "",
    &[("LITELLM_ENABLE_TEAM_STALE_ALIAS_BYPASS", "0")],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::stale_alias_bypass_env_empty(
    "",
    &[("LITELLM_ENABLE_TEAM_STALE_ALIAS_BYPASS", "")],
    Expected::Loaded(Settings {
        credential_overrides: false,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    })
)]
#[case::stale_alias_bypass_env_invalid(
    "",
    &[("LITELLM_ENABLE_TEAM_STALE_ALIAS_BYPASS", "maybe")],
    Expected::InvalidEnvironment
)]
fn loads_settings(#[case] source: &str, #[case] env: &[(&str, &str)], #[case] expected: Expected) {
    match (load(source, env), expected) {
        (Ok(loaded), Expected::Loaded(want)) => assert_eq!(loaded, want),
        (Err(ConfigError::InvalidScalar { path, .. }), Expected::InvalidScalar(want)) => {
            assert_eq!(path, want);
        }
        (Err(ConfigError::Environment { source: failure }), Expected::InvalidEnvironment) => {
            assert_eq!(
                failure,
                EnvError::Bool {
                    name: "LITELLM_ENABLE_TEAM_STALE_ALIAS_BYPASS".to_owned()
                }
            );
        }
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
}

// the negative control: a near miss of a supported key still reaches the unsupported-key report
#[test]
fn reports_unknown_sibling_key() {
    let error = load(
        "general_settings:\n  enforced_paramz: [user]\nlitellm_settings:\n  enable_model_config_credential_overrides: true\n",
        &[],
    )
    .expect_err("an unknown key must fail the load");
    match error {
        ConfigError::Unsupported { paths } => {
            assert_eq!(paths, vec!["general_settings.enforced_paramz".to_owned()]);
        }
        other @ ConfigError::CustomDimensions(_)
        | other @ ConfigError::Environment { .. }
        | other @ ConfigError::Yaml { .. }
        | other @ ConfigError::MissingEnvironment { .. }
        | other @ ConfigError::InvalidFloat { .. }
        | other @ ConfigError::InvalidInteger { .. }
        | other @ ConfigError::InvalidScalar { .. }
        | other @ ConfigError::LoadThread { .. } => panic!("Unsupported expected, got {other:?}"),
    }
}
