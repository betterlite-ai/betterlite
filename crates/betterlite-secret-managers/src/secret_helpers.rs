use crate::secret_types::SecretAccessMode;

#[must_use]
pub fn read_boolean_text(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[must_use]
pub fn normalize_nonempty_secret_str(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(str::to_owned)
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
