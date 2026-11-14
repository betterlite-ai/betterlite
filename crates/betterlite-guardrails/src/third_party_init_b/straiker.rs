use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ThirdPartyInitBError;
use super::{InitResult, ParamsView, decode, unreachable_fallback_text};

mod tests;

const DEFAULT_API_BASE: &str = "https://api.prod.straiker.ai";
const DEFAULT_NAME: &str = "straiker";
const DEFAULT_SOURCE: &str = "LiteLLM Gateway";
const API_VERSIONS: [&str; 2] = ["v1", "v3"];
const FORMAT_HINTS: [&str; 2] = ["anthropic.messages", "openai.chat"];

#[derive(Debug, Clone)]
pub struct StraikerArgs {
    pub name: String,
    pub api_key: SecretString,
    pub api_base: String,
    pub source: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
    pub max_retries: Option<i64>,
    pub initial_backoff: Option<f64>,
    pub max_backoff: Option<f64>,
    pub unreachable_fallback: String,
    pub fail_on_error: bool,
    pub max_payload_bytes: Option<i64>,
    pub custom_headers: Option<BTreeMap<String, String>>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub verbose: Option<bool>,
    pub api_version: Option<String>,
    pub agent_ref: Option<String>,
    pub client: Option<String>,
    pub format_hint: Option<String>,
}

pub fn initialize_straiker(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<StraikerArgs> {
    let (name, api_key, api_base) = required_identity(params, guardrail_name)?;
    let extra = &params.base.extra;
    let view = ParamsView::parse(extra)?;
    Ok(StraikerArgs {
        name,
        api_key,
        api_base,
        source: source_name(extra),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: decode("timeout", view.merged.get("timeout"))?.or(params.base.fields.timeout),
        max_retries: decode("max_retries", view.merged.get("max_retries"))?,
        initial_backoff: decode("initial_backoff", view.merged.get("initial_backoff"))?,
        max_backoff: decode("max_backoff", view.merged.get("max_backoff"))?,
        unreachable_fallback: decode(
            "unreachable_fallback",
            view.merged.get("unreachable_fallback"),
        )?
        .unwrap_or_else(|| {
            unreachable_fallback_text(params.base.fields.unreachable_fallback).to_owned()
        }),
        fail_on_error: decode("fail_on_error", view.merged.get("fail_on_error"))?
            .unwrap_or(params.base.fields.fail_on_error),
        max_payload_bytes: decode("max_payload_bytes", view.merged.get("max_payload_bytes"))?,
        custom_headers: decode("custom_headers", view.merged.get("custom_headers"))?,
        metadata: decode("metadata", view.merged.get("metadata"))?,
        verbose: decode("verbose", view.merged.get("verbose"))?,
        api_version: known_api_version(decode("api_version", view.merged.get("api_version"))?),
        agent_ref: decode("agent_ref", view.merged.get("agent_ref"))?,
        client: decode("client", view.merged.get("client"))?,
        format_hint: checked_format_hint(decode("format_hint", view.merged.get("format_hint"))?)?,
    })
}

fn required_identity(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<(String, SecretString, String)> {
    let api_key = params
        .base
        .fields
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(ThirdPartyInitBError::MissingStraikerApiKey)?;
    let api_base = params
        .base
        .fields
        .api_base
        .clone()
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    Ok((
        guardrail_name.unwrap_or(DEFAULT_NAME).to_owned(),
        api_key,
        api_base,
    ))
}

// LiteLLM's _get_config_value reads default_app first, then source, and only a non-empty string counts
fn source_name(extra: &OpaqueParams) -> String {
    match (
        extra_text(extra, "default_app"),
        extra_text(extra, "source"),
    ) {
        (Some(default_app), _) => default_app,
        (None, Some(source)) => source,
        (None, None) => DEFAULT_SOURCE.to_owned(),
    }
}

fn extra_text(extra: &OpaqueParams, name: &str) -> Option<String> {
    match extra.0.get(name) {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn known_api_version(version: Option<String>) -> Option<String> {
    match version {
        Some(version) if API_VERSIONS.contains(&version.as_str()) => Some(version),
        Some(version) => {
            let shown = format!("{version:?}");
            tracing::warn!(
                "Straiker guardrail: ignoring api_version {shown}, expected 'v1', 'v3' or unset; the route follows the api_key prefix"
            );
            None
        }
        None => None,
    }
}

fn checked_format_hint(hint: Option<String>) -> InitResult<Option<String>> {
    match hint {
        Some(hint) if !FORMAT_HINTS.contains(&hint.as_str()) => {
            Err(ThirdPartyInitBError::InvalidFormatHint { format_hint: hint })
        }
        other => Ok(other),
    }
}
