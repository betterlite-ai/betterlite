use std::collections::BTreeMap;

use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret as _;

use super::error::ConfigBootError;
use super::raw::scalar_text;

const BLOCKED_ENV_KEYS: [&str; 14] = [
    "PATH",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "HOME",
    "USER",
    "SHELL",
    "LOGNAME",
    "NO_PROXY",
    "no_proxy",
];

const MCP_STDIO_ENABLED: &str = "LITELLM_ENABLE_MCP_STDIO";

pub fn env_overlay(
    config: &ConfigMap,
    process_env: &BTreeMap<String, String>,
    secrets: &SecretLookup,
) -> Result<BTreeMap<String, String>, ConfigBootError> {
    let mut overlay = process_env.clone();
    let Some(value) = config.get("environment_variables") else {
        return Ok(overlay);
    };
    if value.is_null() {
        return Ok(overlay);
    }
    let JsonValue::Object(entries) = value else {
        return Err(ConfigBootError::FieldType {
            field: "environment_variables",
            expected: "a mapping",
        });
    };
    for (key, entry) in entries {
        if BLOCKED_ENV_KEYS.contains(&key.as_str()) {
            tracing::warn!("Skipping blocked environment variable key: {key}");
            continue;
        }
        if key.to_uppercase() == MCP_STDIO_ENABLED {
            tracing::warn!(
                "Ignoring {MCP_STDIO_ENABLED} set in the config file. Set it in the proxy's environment instead"
            );
            continue;
        }
        let Some(text) = entry.as_str() else {
            overlay.insert(key.clone(), scalar_text(entry));
            continue;
        };
        if !text.starts_with("os.environ/") {
            overlay.insert(key.clone(), text.to_owned());
            continue;
        }
        let resolved = secrets
            .get_secret_str(&overlay, text, None)
            .map_err(|source| ConfigBootError::Environment { source })?;
        if let Some(secret) = resolved {
            overlay.insert(key.clone(), secret.expose_secret().to_owned());
        }
    }
    Ok(overlay)
}

mod tests;
