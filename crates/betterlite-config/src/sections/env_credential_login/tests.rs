#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::env_credential_login::Settings;

const ENV_LOGIN_REFERENCE: &str =
    "general_settings:\n  disable_env_credential_login: os.environ/DISABLE_ENV_CREDENTIAL_LOGIN\n";
const SSO_BLOCKS_REFERENCE: &str = "general_settings:\n  disable_password_login_when_sso_enabled: os.environ/DISABLE_PASSWORD_LOGIN_WHEN_SSO_ENABLED\n";

fn loaded(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .env_credential_login
}

fn loaded_from(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .env_credential_login)
}

#[rstest]
#[case::both_on(
    "general_settings:\n  disable_env_credential_login: true\n  disable_password_login_when_sso_enabled: true\n",
    true,
    true
)]
#[case::both_off(
    "general_settings:\n  disable_env_credential_login: false\n  disable_password_login_when_sso_enabled: false\n",
    false,
    false
)]
#[case::env_login_only(
    "general_settings:\n  disable_env_credential_login: true\n",
    true,
    false
)]
#[case::sso_blocks_only(
    "general_settings:\n  disable_password_login_when_sso_enabled: true\n",
    false,
    true
)]
fn loads_both_flags(
    #[case] source: &str,
    #[case] env_login_disabled: bool,
    #[case] sso_blocks_password_login: bool,
) {
    let settings = loaded(source);
    assert_eq!(settings.env_login_disabled, env_login_disabled);
    assert_eq!(
        settings.sso_blocks_password_login,
        sso_blocks_password_login
    );
}

#[rstest]
#[case::no_general_settings("model_list: []\n")]
#[case::empty_general_settings("general_settings: {}\n")]
fn flags_default_off(#[case] source: &str) {
    let settings = loaded(source);
    assert!(!settings.env_login_disabled);
    assert!(!settings.sso_blocks_password_login);
}

#[rstest]
#[case::env_login_true(
    "general_settings:\n  disable_env_credential_login: true\n",
    (true, false)
)]
#[case::env_login_false(
    "general_settings:\n  disable_env_credential_login: false\n",
    (false, false)
)]
#[case::env_login_null(
    "general_settings:\n  disable_env_credential_login: null\n",
    (false, false)
)]
#[case::env_login_quoted_true(
    "general_settings:\n  disable_env_credential_login: \"true\"\n",
    (false, false)
)]
#[case::env_login_capital_true(
    "general_settings:\n  disable_env_credential_login: \"True\"\n",
    (false, false)
)]
#[case::env_login_number("general_settings:\n  disable_env_credential_login: 1\n", (false, false))]
#[case::sso_blocks_true(
    "general_settings:\n  disable_password_login_when_sso_enabled: true\n",
    (false, true)
)]
#[case::sso_blocks_false(
    "general_settings:\n  disable_password_login_when_sso_enabled: false\n",
    (false, false)
)]
#[case::sso_blocks_null(
    "general_settings:\n  disable_password_login_when_sso_enabled: null\n",
    (false, false)
)]
#[case::sso_blocks_quoted_true(
    "general_settings:\n  disable_password_login_when_sso_enabled: \"true\"\n",
    (false, false)
)]
#[case::sso_blocks_capital_true(
    "general_settings:\n  disable_password_login_when_sso_enabled: \"True\"\n",
    (false, false)
)]
#[case::sso_blocks_number(
    "general_settings:\n  disable_password_login_when_sso_enabled: 1\n",
    (false, false)
)]
fn only_literal_true(#[case] source: &str, #[case] expected: (bool, bool)) {
    let settings = loaded(source);
    assert_eq!(
        (
            settings.env_login_disabled,
            settings.sso_blocks_password_login
        ),
        expected
    );
}

#[rstest]
#[case::env_login_true(
    ENV_LOGIN_REFERENCE,
    "DISABLE_ENV_CREDENTIAL_LOGIN",
    "true",
    true,
    false
)]
#[case::env_login_false(
    ENV_LOGIN_REFERENCE,
    "DISABLE_ENV_CREDENTIAL_LOGIN",
    "false",
    false,
    false
)]
#[case::sso_blocks_true(
    SSO_BLOCKS_REFERENCE,
    "DISABLE_PASSWORD_LOGIN_WHEN_SSO_ENABLED",
    "true",
    false,
    true
)]
#[case::sso_blocks_false(
    SSO_BLOCKS_REFERENCE,
    "DISABLE_PASSWORD_LOGIN_WHEN_SSO_ENABLED",
    "false",
    false,
    false
)]
fn flags_from_environment(
    #[case] source: &str,
    #[case] variable: &str,
    #[case] value: &str,
    #[case] env_login_disabled: bool,
    #[case] sso_blocks_password_login: bool,
) {
    let settings = loaded_from(source, &[(variable, value)]).unwrap();
    assert_eq!(settings.env_login_disabled, env_login_disabled);
    assert_eq!(
        settings.sso_blocks_password_login,
        sso_blocks_password_login
    );
}

#[rstest]
#[case::env_login(ENV_LOGIN_REFERENCE, "DISABLE_ENV_CREDENTIAL_LOGIN")]
#[case::sso_blocks(SSO_BLOCKS_REFERENCE, "DISABLE_PASSWORD_LOGIN_WHEN_SSO_ENABLED")]
fn flags_from_environment_unset(#[case] source: &str, #[case] variable: &str) {
    let error = loaded_from(source, &[]).unwrap_err();
    assert!(matches!(
        &error,
        ConfigError::MissingEnvironment { variable: missing, .. } if missing == variable
    ));
}
