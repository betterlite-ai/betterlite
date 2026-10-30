use secrecy::SecretString;

use super::{Configured, Entry, FallbackAction, FlaggedAction, InspectionType};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{parse_float, resolve_string};

const DEFAULT_API_BASE: &str = "https://us.api.inspect.aidefense.security.cisco.com";

impl Entry {
    pub(super) fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_key = resolve_api_key(parameters.api_key, &prefix, lookup)?;
        let api_base = resolve_base(parameters.api_base, &prefix, lookup)?;
        let inspection_type = resolve_inspection(parameters.inspection_type, &prefix, lookup)?;
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

fn resolve_base<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = optional(
        configured,
        "CISCO_AI_DEFENSE_API_BASE",
        prefix,
        "api_base",
        lookup,
    )?;
    Ok(value.unwrap_or_else(|| DEFAULT_API_BASE.to_owned()))
}

fn resolve_inspection<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<InspectionType, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = resolve_choice(
        Choice {
            configured,
            variable: "CISCO_AI_DEFENSE_INSPECTION_TYPE",
            prefix,
            name: "inspection_type",
            default: "chat",
            allowed: &["chat", "mcp"],
        },
        lookup,
    )?;
    match value.as_str() {
        "chat" => Ok(InspectionType::Chat),
        "mcp" => Ok(InspectionType::Mcp),
        _ => Err(ConfigError::Unsupported {
            paths: vec![format!("{prefix}.inspection_type")],
        }),
    }
}

fn resolve_flagged_action<L>(
    configured: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<FlaggedAction, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = resolve_choice(
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
    Ok(if value == "monitor" {
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
    let value = resolve_choice(
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
    Ok(if value == "allow" {
        FallbackAction::Allow
    } else {
        FallbackAction::Block
    })
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
    if choice.allowed.contains(&selected.as_str()) {
        return Ok(selected);
    }
    Err(ConfigError::Unsupported {
        paths: vec![format!("{}.{}", choice.prefix, choice.name)],
    })
}

fn resolve_timeout<L>(configured: Option<f64>, prefix: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let timeout = match configured {
        Some(value) => value,
        None => lookup
            .lookup("CISCO_AI_DEFENSE_TIMEOUT")
            .map_or(Ok(10.0_f64), |text| {
                parse_float(&text, &format!("{prefix}.timeout"))
            })?,
    };
    if !(1.0_f64..=60.0_f64).contains(&timeout) {
        tracing::warn!(
            timeout,
            "Cisco AI Defense timeout is outside 1-60 seconds; clamping"
        );
    }
    Ok(timeout.clamp(1.0, 60.0))
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
    let selected = value.or_else(|| lookup.lookup(variable));
    if let Some(text) = selected {
        return resolve_string(text, &format!("{prefix}.{name}"), lookup).map(Some);
    }
    Ok(None)
}
