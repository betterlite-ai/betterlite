use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::guardrail_identity::enums::Role;
use crate::sections::guardrail_identity::item::{
    CallbackArgs, GuardrailItem, default_enabled_roles,
};

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: RawSettings,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawSettings {
    #[serde(default)]
    pub guardrails: Vec<IndexMap<String, RawSpec>>,
}

// GuardrailItem(**spec, guardrail_name=name) raises TypeError for any other key, so an unknown key fails the load
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSpec {
    pub callbacks: Vec<String>,
    #[serde(default)]
    pub default_on: bool,
    #[serde(default)]
    pub logging_only: Option<bool>,
    #[serde(default = "default_enabled_roles")]
    pub enabled_roles: Option<Vec<Role>>,
    #[serde(default)]
    pub callback_args: Option<CallbackArgs>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub items: Vec<GuardrailItem>,
    pub by_name: BTreeMap<String, GuardrailItem>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut settings = Settings::default();
        for entry in self.litellm_settings.guardrails {
            for (guardrail_name, spec) in entry {
                let item = GuardrailItem {
                    callbacks: spec.callbacks,
                    default_on: spec.default_on,
                    logging_only: spec.logging_only,
                    guardrail_name: guardrail_name.clone(),
                    // LiteLLM stores `callback_args or {}`, so null reads as empty
                    callback_args: spec.callback_args.unwrap_or_default(),
                    enabled_roles: spec.enabled_roles,
                };
                settings.items.push(item.clone());
                settings.by_name.insert(guardrail_name, item);
            }
        }
        Ok(settings)
    }
}
