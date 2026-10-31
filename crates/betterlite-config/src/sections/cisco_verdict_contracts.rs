use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;

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
    api_key: Option<SecretString>,
    api_base: Option<String>,
    inspection_type: Option<String>,
    inspect_path: Option<String>,
    enabled_rules: Option<Vec<RawRule>>,
    on_flagged_action: Option<String>,
    fallback_on_error: Option<String>,
    timeout: Option<RawNumber>,
    integration_profile_id: Option<String>,
    integration_profile_version: Option<String>,
    integration_tenant_id: Option<String>,
    integration_type: Option<String>,
}

// LiteLLM's CiscoAIDefenseRule accepts a plain rule name string or a dict carrying rule_name, entity_types, rule_id and classification.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawRule {
    Entry(RuleEntry),
    Name(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RuleEntry {
    rule_name: String,
    entity_types: Option<Vec<String>>,
    rule_id: Option<String>,
    classification: Option<String>,
}

impl RawRule {
    fn resolve(self) -> Rule {
        match self {
            Self::Name(rule_name) => Rule {
                rule_name,
                entity_types: Vec::new(),
                rule_id: None,
                classification: None,
            },
            Self::Entry(RuleEntry {
                rule_name,
                entity_types,
                rule_id,
                classification,
            }) => Rule {
                rule_name,
                entity_types: entity_types.unwrap_or_default(),
                rule_id,
                classification,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub rule_name: String,
    pub entity_types: Vec<String>,
    pub rule_id: Option<String>,
    pub classification: Option<String>,
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
    pub enabled_rules: Vec<Rule>,
    pub on_flagged_action: FlaggedAction,
    pub fallback_on_error: FallbackAction,
    pub timeout: f64,
    pub integration_profile_id: Option<String>,
    pub integration_profile_version: Option<String>,
    pub integration_tenant_id: Option<String>,
    pub integration_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectionType {
    Chat,
    Mcp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlaggedAction {
    Block,
    Monitor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
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
            if entry.litellm_params.guardrail == "cisco_ai_defense" {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

mod resolve;
mod tests;
