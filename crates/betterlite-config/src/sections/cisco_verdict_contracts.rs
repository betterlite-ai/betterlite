use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{parse_float, resolve_string};

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
        let api_key = resolve_api_key(parameters.api_key, &prefix, lookup)?;
        let api_base = optional(
            parameters.api_base,
            "CISCO_AI_DEFENSE_API_BASE",
            &prefix,
            "api_base",
            lookup,
        )?
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let inspection = resolve_choice(
            Choice {
                configured: parameters.inspection_type,
                variable: "CISCO_AI_DEFENSE_INSPECTION_TYPE",
                prefix: &prefix,
                name: "inspection_type",
                default: "chat",
                allowed: &["chat", "mcp"],
            },
            lookup,
        )?;
        let inspection_type = match inspection.as_str() {
            "chat" => InspectionType::Chat,
            "mcp" => InspectionType::Mcp,
            _ => {
                return Err(ConfigError::Unsupported {
                    paths: vec![format!("{prefix}.inspection_type")],
                });
            }
        };
        let inspect_path = parameters
            .inspect_path
            .unwrap_or_else(|| match inspection_type {
                InspectionType::Chat => "/api/v1/inspect/chat".to_owned(),
                InspectionType::Mcp => "/api/v1/inspect/mcp".to_owned(),
            });
        let on_flagged_action =
            resolve_flagged_action(parameters.on_flagged_action, &prefix, lookup)?;
        let fallback_on_error = resolve_fallback(parameters.fallback_on_error, &prefix, lookup)?;
        let timeout = resolve_timeout(parameters.timeout, &prefix, lookup)?;
        Ok(Configured {
            guardrail_name: self
                .guardrail_name
                .unwrap_or_else(|| "cisco-ai-defense".to_owned()),
            api_key,
            api_base,
            inspection_type,
            inspect_path,
            enabled_rules: parameters.enabled_rules,
            on_flagged_action,
            fallback_on_error,
            timeout,
            integration_profile_id: parameters.integration_profile_id,
            integration_profile_version: parameters.integration_profile_version,
            integration_tenant_id: parameters.integration_tenant_id,
            integration_type: parameters.integration_type,
        })
    }
}

fn resolve_api_key<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let selected = configured
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup("CISCO_AI_DEFENSE_API_KEY"))
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "CISCO_AI_DEFENSE_API_KEY".to_owned(),
            path: format!("{prefix}.api_key"),
        })?;
    let secret = resolve_string(selected, &format!("{prefix}.api_key"), lookup)?;
    Ok(SecretString::from(secret))
}

struct Choice<'choice> {
    configured: Option<String>,
    variable: &'choice str,
    prefix: &'choice str,
    name: &'choice str,
    default: &'choice str,
    allowed: &'choice [&'choice str],
}

fn resolve_choice<L>(choice: Choice<'_>, lookup: &L) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let selected = optional(
        choice.configured,
        choice.variable,
        choice.prefix,
        choice.name,
        lookup,
    )?
    .unwrap_or_else(|| choice.default.to_owned());
    choice_value(
        &selected,
        choice.allowed,
        &format!("{}.{}", choice.prefix, choice.name),
    )
}

fn choice_value(value: &str, allowed: &[&str], path: &str) -> Result<String, ConfigError> {
    if allowed.contains(&value) {
        return Ok(value.to_owned());
    }
    Err(ConfigError::Unsupported {
        paths: vec![path.to_owned()],
    })
}

fn resolve_flagged_action<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<FlaggedAction, ConfigError>
where
    L: EnvironmentLookup,
{
    let flagged_choice = resolve_choice(
        Choice {
            configured,
            variable: "CISCO_AI_DEFENSE_ON_FLAGGED_ACTION",
            prefix,
            name: "on_flagged_action",
            default: "block",
            allowed: &["block", "monitor"],
        },
        lookup,
    )?;
    Ok(if flagged_choice == "monitor" {
        FlaggedAction::Monitor
    } else {
        FlaggedAction::Block
    })
}

fn resolve_fallback<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<FallbackAction, ConfigError>
where
    L: EnvironmentLookup,
{
    let action = resolve_choice(
        Choice {
            configured,
            variable: "CISCO_AI_DEFENSE_FALLBACK_ON_ERROR",
            prefix,
            name: "fallback_on_error",
            default: "block",
            allowed: &["allow", "block"],
        },
        lookup,
    )?;
    Ok(if action == "allow" {
        FallbackAction::Allow
    } else {
        FallbackAction::Block
    })
}

fn resolve_timeout<L>(configured: Option<f64>, prefix: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let timeout = if configured.is_none()
        && let Some(text) = lookup.lookup("CISCO_AI_DEFENSE_TIMEOUT")
    {
        parse_float(&text, &format!("{prefix}.timeout"))?
    } else {
        configured.unwrap_or(10.0_f64)
    };
    if !(1.0_f64..=60.0_f64).contains(&timeout) {
        tracing::warn!(
            timeout,
            "Cisco AI Defense timeout is outside 1-60 seconds; clamping"
        );
    }
    Ok(timeout.clamp(1.0_f64, 60.0_f64))
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
        .map(|text| resolve_string(text, &format!("{prefix}.{name}"), lookup))
        .transpose()
}

mod tests;
