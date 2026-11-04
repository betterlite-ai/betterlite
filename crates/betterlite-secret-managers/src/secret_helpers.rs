use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_types::SecretAccessMode;

pub fn read_boolean_text(
    name: &str,
    value: Option<&str>,
) -> Result<Option<bool>, betterlite_env::EnvError> {
    value
        .map(|text| betterlite_env::flag(name, text))
        .transpose()
        .map(Option::flatten)
}

#[must_use]
pub fn normalize_nonempty_secret_str(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .map(|secret| {
            secret.expose_secret().trim_matches(|character: char| {
                character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
            })
        })
        .filter(|candidate| !candidate.is_empty())
        .map(SecretString::from)
}

#[derive(Debug, Clone, Copy)]
pub struct SecretManagerReadState<'borrowed> {
    pub client_exists: bool,
    pub settings_exist: bool,
    pub access_mode: SecretAccessMode,
    pub hosted_keys: Option<&'borrowed [String]>,
}

#[must_use]
pub fn manager_read_is_consulted(state: SecretManagerReadState<'_>, secret_name: &str) -> bool {
    state.hosted_keys.map_or_else(
        || manager_read_is_enabled(state),
        |keys| {
            manager_read_is_enabled(state)
                && keys.iter().any(|key| {
                    key == secret_name
                        .strip_prefix("os.environ/")
                        .unwrap_or(secret_name)
                })
        },
    )
}

#[must_use]
pub const fn manager_read_is_enabled(state: SecretManagerReadState<'_>) -> bool {
    state.client_exists
        && state.settings_exist
        && matches!(
            state.access_mode,
            SecretAccessMode::ReadOnly | SecretAccessMode::ReadAndWrite
        )
}

#[path = "secret_helpers/tests.rs"]
mod tests;
