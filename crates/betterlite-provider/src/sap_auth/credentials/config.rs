use std::path::PathBuf;

use betterlite_prompts::arguments::json::{Json, Object};

use super::{CONFIG_FILE_ENV_VAR, HOME_PATH_ENV_VAR, PROFILE_ENV_VAR};
use crate::sap_auth::environment::SapEnvironment;
use crate::sap_auth::error::SapAuthError;

const DEFAULT_PROFILE_CONFIG: &str = "config.json";

pub(super) fn load_config(environment: &dyn SapEnvironment) -> Result<Json, SapAuthError> {
    let home = environment
        .var(HOME_PATH_ENV_VAR)
        .map_or_else(|| environment.home_dir().join(".aicore"), PathBuf::from);
    let profile = environment.var(PROFILE_ENV_VAR);
    let config_env = environment.var(CONFIG_FILE_ENV_VAR);
    let explicit_config = config_env.as_deref().is_some_and(|value| !value.is_empty());
    let named_profile = profile
        .as_deref()
        .is_some_and(|value| !value.is_empty() && value != "default");
    let path = match config_env.as_deref() {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home.join(profile_file_name(profile.as_deref())),
    };
    if environment.file_exists(&path) {
        let text = environment
            .read_file(&path)
            .map_err(|source| SapAuthError::ConfigRead {
                path: path.clone(),
                source,
            })?;
        return serde_json::from_str::<Json>(&text)
            .map_err(|source| SapAuthError::ConfigNotJson { path, source });
    }
    if explicit_config || named_profile {
        return Err(SapAuthError::ConfigFileNotFound { path, home });
    }
    Ok(Json::Object(Object::new()))
}

fn profile_file_name(profile: Option<&str>) -> String {
    match profile {
        None | Some("" | "default") => DEFAULT_PROFILE_CONFIG.to_owned(),
        Some(name) => format!("config_{name}.json"),
    }
}
