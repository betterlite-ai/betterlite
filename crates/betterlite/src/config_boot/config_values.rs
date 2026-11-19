use betterlite_config::credentials::CredentialItem;
use betterlite_config::sections::includes::ConfigMap;

use super::error::ConfigBootError;

const JWT_KEYS: [&str; 2] = ["enable_jwt_auth", "litellm_jwtauth"];

#[must_use]
#[must_use]
pub fn misplaced_jwt_keys(config: &ConfigMap) -> Vec<&'static str> {
    let misplaced: Vec<&'static str> = JWT_KEYS
        .into_iter()
        .filter(|key| config.contains_key(*key))
        .collect();
    if misplaced.is_empty() {
        return misplaced;
    }
    tracing::warn!(
        "Ignoring top-level config key(s) {}. JWT auth settings must live under 'general_settings' (e.g. general_settings.enable_jwt_auth, general_settings.litellm_jwtauth); placed at the top level they are silently dropped and JWT auth (and JWT-to-virtual-key mapping) will not engage.",
        misplaced.join(", ")
    );
    misplaced
}

pub fn credential_list(config: &ConfigMap) -> Result<Vec<CredentialItem>, ConfigBootError> {
    let Some(value) = config.get("credential_list") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value::<Vec<CredentialItem>>(value.clone())
        .map_err(|source| ConfigBootError::CredentialList { source })
}

mod tests;
