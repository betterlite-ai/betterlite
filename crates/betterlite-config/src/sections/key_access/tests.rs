#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{KeyGeneration, PersonalKeyGeneration, Settings, TeamKeyGeneration};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, lookup: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let variables = lookup
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<String, String>>();
    ConfigLoader { lookup: variables }
        .load(source)
        .map(|config| config.sections.key_access)
}

fn roles(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn loads_settings() {
    let source = "general_settings:\n  disable_custom_api_keys: os.environ/CUSTOM_KEYS_OFF\nlitellm_settings:\n  key_generation_settings:\n    team_key_generation:\n      allowed_team_member_roles: [admin]\n      required_params: [tags]\n    personal_key_generation:\n      allowed_user_roles: [proxy_admin, internal_user]\n      required_params: [key_alias, duration]\n";
    let settings = load(source, &[("CUSTOM_KEYS_OFF", " TRUE ")]).unwrap();
    assert_eq!(
        settings,
        Settings {
            disable_custom_api_keys: true,
            key_generation: KeyGeneration {
                configured: true,
                team: TeamKeyGeneration {
                    allowed_team_member_roles: Some(roles(&["admin"])),
                    required_params: Some(roles(&["tags"])),
                },
                personal: Some(PersonalKeyGeneration {
                    allowed_user_roles: Some(roles(&["proxy_admin", "internal_user"])),
                    required_params: Some(roles(&["key_alias", "duration"])),
                }),
            },
        }
    );
    let missing = load(source, &[]).unwrap_err();
    assert!(matches!(
        missing,
        ConfigError::MissingEnvironment { variable, path }
        if variable == "CUSTOM_KEYS_OFF" && path == "general_settings.disable_custom_api_keys"
    ));
    let wrong_shape = "litellm_settings:\n  key_generation_settings:\n    team_key_generation:\n      allowed_team_member_roles: admin\n";
    load(wrong_shape, &[]).unwrap_err();
}

#[rstest]
#[case::unset("{}", KeyGeneration::default())]
#[case::null(
    "litellm_settings:\n  key_generation_settings: null\n",
    KeyGeneration::default()
)]
#[case::empty_mapping(
    "litellm_settings:\n  key_generation_settings: {}\n",
    KeyGeneration { configured: true, team: TeamKeyGeneration::default(), personal: None }
)]
#[case::team_null(
    "litellm_settings:\n  key_generation_settings:\n    team_key_generation: null\n    personal_key_generation: null\n",
    KeyGeneration { configured: true, team: TeamKeyGeneration::default(), personal: None }
)]
#[case::configured_team_roles_missing(
    "litellm_settings:\n  key_generation_settings:\n    team_key_generation: {required_params: []}\n    personal_key_generation: {}\n",
    KeyGeneration {
        configured: true,
        team: TeamKeyGeneration { allowed_team_member_roles: None, required_params: Some(Vec::new()) },
        personal: Some(PersonalKeyGeneration { allowed_user_roles: None, required_params: None }),
    }
)]
fn generation_defaults(#[case] source: &str, #[case] expected: KeyGeneration) {
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.key_generation, expected);
    assert!(!settings.disable_custom_api_keys);
    assert_eq!(
        TeamKeyGeneration::default().allowed_team_member_roles,
        Some(roles(&["admin", "user"]))
    );
}

#[rstest]
#[case::absent("{}", &[], false)]
#[case::null("disable_custom_api_keys: null", &[], false)]
#[case::bool_true("disable_custom_api_keys: true", &[], true)]
#[case::bool_false("disable_custom_api_keys: false", &[], false)]
#[case::text_true("disable_custom_api_keys: 'true'", &[], true)]
#[case::text_upper("disable_custom_api_keys: 'TRUE'", &[], true)]
#[case::text_padded("disable_custom_api_keys: ' true '", &[], false)]
#[case::text_one("disable_custom_api_keys: '1'", &[], false)]
#[case::text_yes("disable_custom_api_keys: 'yes'", &[], false)]
#[case::text_empty("disable_custom_api_keys: ''", &[], false)]
#[case::number_one("disable_custom_api_keys: 1", &[], true)]
#[case::number_zero("disable_custom_api_keys: 0", &[], false)]
#[case::number_fraction("disable_custom_api_keys: 0.5", &[], true)]
#[case::empty_list("disable_custom_api_keys: []", &[], false)]
#[case::list("disable_custom_api_keys: [0]", &[], true)]
#[case::empty_mapping("disable_custom_api_keys: {}", &[], false)]
#[case::env_padded("disable_custom_api_keys: os.environ/FLAG", &[("FLAG", " True ")], true)]
#[case::env_false("disable_custom_api_keys: os.environ/FLAG", &[("FLAG", "false")], false)]
#[case::env_one("disable_custom_api_keys: os.environ/FLAG", &[("FLAG", "1")], false)]
#[case::env_empty("disable_custom_api_keys: os.environ/FLAG", &[("FLAG", "")], false)]
fn custom_bool_coercion(
    #[case] general: &str,
    #[case] lookup: &[(&str, &str)],
    #[case] expected: bool,
) {
    let source = if general == "{}" {
        "{}".to_owned()
    } else {
        format!("general_settings:\n  {general}\n")
    };
    let settings = load(&source, lookup).unwrap();
    assert_eq!(settings.disable_custom_api_keys, expected);
    assert!(!settings.key_generation.configured);
}
