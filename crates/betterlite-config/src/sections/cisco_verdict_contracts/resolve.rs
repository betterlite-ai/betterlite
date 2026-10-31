use secrecy::{ExposeSecret as _, SecretString};

use super::{Configured, Entry, FallbackAction, FlaggedAction, InspectionType};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};

const DEFAULT_API_BASE: &str = "https://us.api.inspect.aidefense.security.cisco.com";

impl Entry {
    pub(super) fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_key = resolve_api_key(parameters.api_key, &prefix, lookup)?;
        let (api_base, inspection_type) = resolve_endpoint(
            parameters.api_base,
            parameters.inspection_type,
            &prefix,
            lookup,
        )?;
        let inspect_path = resolve_inspect_path(parameters.inspect_path, inspection_type);
        let (on_flagged_action, fallback_on_error, timeout) = resolve_actions(
            parameters.on_flagged_action,
            parameters.fallback_on_error,
            parameters.timeout,
            &prefix,
            lookup,
        )?;
        Ok(Configured {
            guardrail_name: self
                .guardrail_name
                .unwrap_or_else(|| "cisco-ai-defense".to_owned()),
            api_key,
            api_base,
            inspection_type,
            inspect_path,
            enabled_rules: parameters
                .enabled_rules
                .unwrap_or_default()
                .into_iter()
                .map(super::RawRule::resolve)
                .collect(),
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
    configured: Option<SecretString>,
    prefix: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let non_empty = configured.filter(|value| !value.expose_secret().is_empty());
    let secret = non_empty
        .unwrap_or_else(|| SecretString::from("os.environ/CISCO_AI_DEFENSE_API_KEY".to_owned()));
    resolve_secret(secret, &format!("{prefix}.api_key"), lookup)
}

// Resolves an `os.environ/NAME` indirection without ever copying the secret into a plain String.
fn resolve_secret<L>(
    secret: SecretString,
    path: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(suffix) = secret.expose_secret().strip_prefix("os.environ/") else {
        return Ok(secret);
    };
    let variable = suffix.to_owned();
    lookup
        .lookup(&variable)
        .map(SecretString::from)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable,
            path: path.to_owned(),
        })
}

fn resolve_endpoint<L>(
    configured_base: Option<String>,
    configured_inspection: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<(String, InspectionType), ConfigError>
where
    L: EnvironmentLookup,
{
    let api_base = optional(
        configured_base,
        "CISCO_AI_DEFENSE_API_BASE",
        prefix,
        "api_base",
        lookup,
    )?
    .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let inspection = resolve_choice(
        Choice {
            configured: configured_inspection,
            variable: "CISCO_AI_DEFENSE_INSPECTION_TYPE",
            prefix,
            name: "inspection_type",
            default: "chat",
            allowed: &["chat", "mcp"],
        },
        lookup,
    )?;
    let inspection_type = if inspection == "chat" {
        InspectionType::Chat
    } else {
        InspectionType::Mcp
    };
    Ok((api_base, inspection_type))
}

fn resolve_inspect_path(
    configured_path: Option<String>,
    inspection_type: InspectionType,
) -> String {
    if let Some(path) = configured_path {
        return path;
    }
    match inspection_type {
        InspectionType::Chat => "/api/v1/inspect/chat".to_owned(),
        InspectionType::Mcp => "/api/v1/inspect/mcp".to_owned(),
    }
}

fn resolve_actions<L>(
    configured_flagged: Option<String>,
    configured_fallback: Option<String>,
    configured_timeout: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<(FlaggedAction, FallbackAction, f64), ConfigError>
where
    L: EnvironmentLookup,
{
    let flagged = resolve_choice(
        Choice {
            configured: configured_flagged,
            variable: "CISCO_AI_DEFENSE_ON_FLAGGED_ACTION",
            prefix,
            name: "on_flagged_action",
            default: "block",
            allowed: &["block", "monitor"],
        },
        lookup,
    )?;
    let fallback = resolve_choice(
        Choice {
            configured: configured_fallback,
            variable: "CISCO_AI_DEFENSE_FALLBACK_ON_ERROR",
            prefix,
            name: "fallback_on_error",
            default: "block",
            allowed: &["allow", "block"],
        },
        lookup,
    )?;
    let flagged_action = if flagged == "monitor" {
        FlaggedAction::Monitor
    } else {
        FlaggedAction::Block
    };
    let fallback_action = if fallback == "allow" {
        FallbackAction::Allow
    } else {
        FallbackAction::Block
    };
    let timeout = resolve_timeout(configured_timeout, prefix, lookup)?;
    Ok((flagged_action, fallback_action, timeout))
}

fn resolve_timeout<L>(
    configured_timeout: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let timeout_path = format!("{prefix}.timeout");
    let configured = resolve_number(configured_timeout, &timeout_path, lookup, parse_float)?;
    let timeout = match configured {
        Some(value) => value,
        None => lookup
            .lookup("CISCO_AI_DEFENSE_TIMEOUT")
            .map_or(Ok(10.0_f64), |text| parse_float(&text, &timeout_path))?,
    };
    let clamped = timeout.clamp(1.0_f64, 60.0_f64);
    let out_of_range = !(1.0_f64..=60.0_f64).contains(&timeout);
    if out_of_range {
        tracing::warn!(
            timeout,
            "Cisco AI Defense timeout is outside 1-60 seconds; clamping"
        );
    }
    Ok(clamped)
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
    )?;
    let Some(candidate) = selected else {
        return Ok(choice.default.to_owned());
    };
    if choice.allowed.contains(&candidate.as_str()) {
        return Ok(candidate);
    }
    tracing::warn!(
        value = candidate,
        setting = choice.name,
        default = choice.default,
        "invalid Cisco AI Defense setting value; falling back to default"
    );
    Ok(choice.default.to_owned())
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
