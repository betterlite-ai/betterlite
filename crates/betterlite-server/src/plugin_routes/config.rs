use std::sync::{PoisonError, RwLock};

use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::PluginError;

#[derive(Clone, Debug, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub display_name: Option<String>,
    pub url: String,
    pub plugin_key: Option<SecretString>,
}

// general_settings.plugins, swapped whole by replace_from_settings; readers lock `plugins` directly
#[derive(Debug, Default)]
pub struct PluginRegistry {
    pub plugins: RwLock<IndexMap<String, PluginConfig>>,
}

impl PluginRegistry {
    // LiteLLM's register_plugins_from_config: validate every entry, then replace (never merge) at once
    pub fn replace_from_settings(&self, settings: &JsonObject) -> Result<(), PluginError> {
        let mut next = IndexMap::new();
        if let Some(JsonValue::Array(entries)) = settings.get("plugins") {
            for (index, entry) in entries.iter().enumerate() {
                let config: PluginConfig = serde_json::from_value(entry.clone())
                    .map_err(|source| PluginError::InvalidEntry { index, source })?;
                next.insert(config.name.clone(), config);
            }
        }
        let mut plugins = self.plugins.write().unwrap_or_else(PoisonError::into_inner);
        *plugins = next;
        Ok(())
    }
}

mod tests;
