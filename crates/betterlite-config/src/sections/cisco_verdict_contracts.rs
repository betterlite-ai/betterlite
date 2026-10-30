use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
struct Entry {
    guardrail_name: Option<String>,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_key: Option<String>,
    api_base: Option<String>,
    inspection_type: Option<String>,
    inspect_path: Option<String>,
    enabled_rules: Option<Vec<Rule>>,
    on_flagged_action: Option<String>,
    fallback_on_error: Option<String>,
    timeout: Option<f64>,
    integration_profile_id: Option<String>,
    integration_profile_version: Option<String>,
    integration_tenant_id: Option<String>,
    integration_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub rule_name: String,
    #[serde(default)]
    pub entity_types: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub inspection_type: InspectionType,
    pub inspect_path: String,
    pub enabled_rules: Option<Vec<Rule>>,
    pub on_flagged_action: FlaggedAction,
    pub fallback_on_error: FallbackAction,
    pub timeout: f64,
    pub integration_profile_id: Option<String>,
    pub integration_profile_version: Option<String>,
    pub integration_tenant_id: Option<String>,
    pub integration_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionType {
    Chat,
    Mcp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlaggedAction {
    Block,
    Monitor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackAction {
    Allow,
    Block,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if ["cisco_ai_defense", "cisco-ai-defense"]
                .contains(&entry.litellm_params.guardrail.as_str())
            {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

mod resolve;
mod tests;
