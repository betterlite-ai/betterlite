use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::{self, value::StrDeserializer};

use super::{Configured, Entry, FallbackAction, FlaggedAction, InspectionType};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};

const DEFAULT_API_BASE: &str = "https://us.api.inspect.aidefense.security.cisco.com";
const DEFAULT_CHAT_INSPECT_PATH: &str = "/api/v1/inspect/chat";
const DEFAULT_MCP_INSPECT_PATH: &str = "/api/v1/inspect/mcp";
const DEFAULT_TIMEOUT_SECONDS: f64 = 10.0_f64;

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
    let Some(suffix) = secret.expose_secret().strip_prefix("os.environ/") else {
        return Ok(secret);
    };
    let variable = suffix.to_owned();
    lookup
        .lookup(&variable)
        .map(SecretString::from)
        .filter(|resolved| !resolved.expose_secret().is_empty())
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable,
            path: format!("{prefix}.api_key"),
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
    let api_base = resolve_api_base(configured_base, prefix, lookup)?;
    let inspection_type = resolve_enum_choice(
        Choice {
            configured: configured_inspection,
            variable: "CISCO_AI_DEFENSE_INSPECTION_TYPE",
            prefix,
            name: "inspection_type",
        },
        InspectionType::Chat,
        lookup,
    )?;
    Ok((api_base, inspection_type))
}

fn resolve_api_base<L>(
    configured_base: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{prefix}.api_base");
    let configured = configured_base
        .filter(|value| !value.is_empty())
        .map(|value| resolve_string(value, &path, lookup))
        .transpose()?
        .filter(|value| !value.is_empty());
    let base = configured.unwrap_or_else(|| {
        lookup
            .lookup("CISCO_AI_DEFENSE_API_BASE")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
    });
    Ok(base.trim_end_matches('/').to_owned())
}

fn resolve_inspect_path(
    configured_path: Option<String>,
    inspection_type: InspectionType,
) -> String {
    let configured = configured_path.filter(|value| !value.is_empty());
    match configured {
        Some(value) if value.starts_with('/') => value,
        Some(value) => format!("/{value}"),
        None => default_inspect_path(inspection_type).to_owned(),
    }
}

const fn default_inspect_path(inspection_type: InspectionType) -> &'static str {
    match inspection_type {
        InspectionType::Chat => DEFAULT_CHAT_INSPECT_PATH,
        InspectionType::Mcp => DEFAULT_MCP_INSPECT_PATH,
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
    let flagged_action = resolve_enum_choice(
        Choice {
            configured: configured_flagged,
            variable: "CISCO_AI_DEFENSE_ON_FLAGGED_ACTION",
            prefix,
            name: "on_flagged_action",
        },
        FlaggedAction::Block,
        lookup,
    )?;
    let fallback_action = resolve_enum_choice(
        Choice {
            configured: configured_fallback,
            variable: "CISCO_AI_DEFENSE_FALLBACK_ON_ERROR",
            prefix,
            name: "fallback_on_error",
        },
        FallbackAction::Block,
        lookup,
    )?;
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
    let configured = match resolve_number(configured_timeout, &timeout_path, lookup, parse_float) {
        Ok(value) => value,
        Err(ConfigError::InvalidFloat { .. }) => Some(warn_invalid_timeout()),
        Err(other) => return Err(other),
    };
    let timeout = configured.unwrap_or_else(|| {
        lookup
            .lookup("CISCO_AI_DEFENSE_TIMEOUT")
            .map_or(DEFAULT_TIMEOUT_SECONDS, |text| {
                parse_float(&text, &timeout_path).unwrap_or_else(|_| warn_invalid_timeout())
            })
    });
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

fn warn_invalid_timeout() -> f64 {
    tracing::warn!(
        setting = "timeout",
        default = DEFAULT_TIMEOUT_SECONDS,
        "invalid Cisco AI Defense timeout value; falling back to default"
    );
    DEFAULT_TIMEOUT_SECONDS
}

struct Choice<'choice> {
    configured: Option<String>,
    variable: &'choice str,
    prefix: &'choice str,
    name: &'choice str,
}

fn resolve_enum_choice<T, L>(choice: Choice<'_>, default: T, lookup: &L) -> Result<T, ConfigError>
where
    T: for<'de> Deserialize<'de>,
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
        return Ok(default);
    };
    let deserializer = StrDeserializer::<de::value::Error>::new(candidate.as_str());
    Ok(T::deserialize(deserializer).unwrap_or_else(|_| {
        tracing::warn!(
            value = candidate,
            setting = choice.name,
            "invalid Cisco AI Defense setting value; falling back to default"
        );
        default
    }))
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
