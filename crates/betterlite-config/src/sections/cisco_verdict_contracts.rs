use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

const DEFAULT_API_BASE: &str = "https://us.api.inspect.aidefense.security.cisco.com";

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
            } else {
                return Err(ConfigError::Unsupported {
                    paths: vec![format!("guardrails.{index}.litellm_params.guardrail")],
                });
            }
        }
        Ok(Settings { guardrails })
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let parameters = self.litellm_params;
        let prefix = format!("guardrails.{index}.litellm_params");
        let api_key = parameters
            .api_key
            .filter(|value| !value.is_empty())
            .or_else(|| lookup.lookup("CISCO_AI_DEFENSE_API_KEY"))
            .ok_or_else(|| ConfigError::MissingEnvironment {
                variable: "CISCO_AI_DEFENSE_API_KEY".to_owned(),
                path: format!("{prefix}.api_key"),
            })?;
        let api_key = SecretString::from(resolve_string(
            api_key,
            &format!("{prefix}.api_key"),
            lookup,
        )?);
        let api_base = optional(
            parameters.api_base,
            "CISCO_AI_DEFENSE_API_BASE",
            &prefix,
            "api_base",
            lookup,
        )?
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let inspection = optional(
            parameters.inspection_type,
            "CISCO_AI_DEFENSE_INSPECTION_TYPE",
            &prefix,
            "inspection_type",
            lookup,
        )?
        .unwrap_or_else(|| "chat".to_owned());
        let inspection = choice(
            &inspection,
            &["chat", "mcp"],
            &format!("{prefix}.inspection_type"),
        )?;
        let inspection_type = if inspection == "mcp" {
            InspectionType::Mcp
        } else {
            InspectionType::Chat
        };
        let inspect_path = parameters
            .inspect_path
            .unwrap_or_else(|| match inspection_type {
                InspectionType::Chat => "/api/v1/inspect/chat".to_owned(),
                InspectionType::Mcp => "/api/v1/inspect/mcp".to_owned(),
            });
        let flagged = optional(
            parameters.on_flagged_action,
            "CISCO_AI_DEFENSE_ON_FLAGGED_ACTION",
            &prefix,
            "on_flagged_action",
            lookup,
        )?
        .unwrap_or_else(|| "block".to_owned());
        let flagged = choice(
            &flagged,
            &["block", "monitor"],
            &format!("{prefix}.on_flagged_action"),
        )?;
        let fallback = optional(
            parameters.fallback_on_error,
            "CISCO_AI_DEFENSE_FALLBACK_ON_ERROR",
            &prefix,
            "fallback_on_error",
            lookup,
        )?
        .unwrap_or_else(|| "block".to_owned());
        let fallback = choice(
            &fallback,
            &["allow", "block"],
            &format!("{prefix}.fallback_on_error"),
        )?;
        let timeout = match parameters.timeout {
            Some(timeout) => timeout,
            None => optional(None, "CISCO_AI_DEFENSE_TIMEOUT", &prefix, "timeout", lookup)?
                .map_or(Ok(10.0), |value| {
                    crate::resolve::parse_float(&value, &format!("{prefix}.timeout"))
                })?,
        };
        if !(1.0..=60.0).contains(&timeout) {
            tracing::warn!(
                timeout,
                "Cisco AI Defense timeout is outside 1-60 seconds; clamping"
            );
        }
        let timeout = timeout.clamp(1.0, 60.0);
        Ok(Configured {
            guardrail_name: self
                .guardrail_name
                .unwrap_or_else(|| "cisco-ai-defense".to_owned()),
            api_key,
            api_base,
            inspection_type,
            inspect_path,
            enabled_rules: parameters.enabled_rules,
            on_flagged_action: if flagged == "monitor" {
                FlaggedAction::Monitor
            } else {
                FlaggedAction::Block
            },
            fallback_on_error: if fallback == "allow" {
                FallbackAction::Allow
            } else {
                FallbackAction::Block
            },
            timeout,
            integration_profile_id: parameters.integration_profile_id,
            integration_profile_version: parameters.integration_profile_version,
            integration_tenant_id: parameters.integration_tenant_id,
            integration_type: parameters.integration_type,
        })
    }
}

fn optional<L>(
    value: Option<String>,
    variable: &str,
    prefix: &str,
    name: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = value.or_else(|| lookup.lookup(variable));
    resolved
        .map(|value| resolve_string(value, &format!("{prefix}.{name}"), lookup))
        .transpose()
}

fn choice(value: &str, allowed: &[&str], path: &str) -> Result<String, ConfigError> {
    if allowed.contains(&value) {
        return Ok(value.to_owned());
    }
    Err(ConfigError::Unsupported {
        paths: vec![path.to_owned()],
    })
}

mod tests;
