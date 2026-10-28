use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{parse_float, resolve_string};

mod tests;

pub const UI_FRIENDLY_NAME: &str = "Gray Swan Cygnal";
const DEFAULT_API_BASE: &str = "https://api.grayswan.ai";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OnFlaggedAction {
    Block,
    #[default]
    Monitor,
    Passthrough,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
struct Entry {
    guardrail_name: String,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    on_flagged_action: Option<String>,
    violation_threshold: Option<String>,
    reasoning_mode: Option<String>,
    categories: Option<Vec<String>>,
    policy_id: Option<String>,
    fail_open: Option<bool>,
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
    pub on_flagged_action: OnFlaggedAction,
    pub violation_threshold: f64,
    pub reasoning_mode: Option<String>,
    pub categories: Option<Vec<String>>,
    pub policy_id: Option<String>,
    pub fail_open: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "grayswan" {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

const DEFAULT_VIOLATION_THRESHOLD: f64 = 0.5_f64;

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let api_key = resolve_api_key(parameters.api_key, &prefix, lookup)?;
        let api_base = resolve_api_base(parameters.api_base, &prefix, lookup)?;
        let violation_threshold =
            resolve_violation_threshold(parameters.violation_threshold, &prefix, lookup)?;
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base,
            on_flagged_action: resolve_on_flagged_action(parameters.on_flagged_action.as_deref()),
            violation_threshold,
            reasoning_mode: resolve_reasoning_mode(parameters.reasoning_mode, lookup),
            categories: parameters.categories,
            policy_id: parameters.policy_id,
            fail_open: parameters.fail_open.unwrap_or(true),
        })
    }
}

fn resolve_api_key<L>(
    api_key: Option<SecretString>,
    prefix: &str,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let key_text = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .map(|key| key.expose_secret().to_owned())
        .or_else(|| lookup.lookup("GRAYSWAN_API_KEY"))
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "GRAYSWAN_API_KEY".to_owned(),
            path: format!("{prefix}.api_key"),
        })?;
    Ok(SecretString::from(resolve_string(
        key_text,
        &format!("{prefix}.api_key"),
        lookup,
    )?))
}

fn resolve_api_base<L>(api_base: Option<String>, prefix: &str, lookup: &L) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let base = api_base.filter(|value| !value.is_empty()).map_or_else(
        || {
            Ok(lookup
                .lookup("GRAYSWAN_API_BASE")
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()))
        },
        |value| resolve_string(value, &format!("{prefix}.api_base"), lookup),
    )?;
    Ok(base.trim_end_matches('/').to_owned())
}

fn resolve_violation_threshold<L>(
    value: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let threshold = value.map_or_else(
        || {
            Ok(lookup
                .lookup("GRAYSWAN_VIOLATION_THRESHOLD")
                .and_then(|text| text.parse::<f64>().ok())
                .unwrap_or(DEFAULT_VIOLATION_THRESHOLD))
        },
        |text| parse_float(&text, &format!("{prefix}.violation_threshold")),
    )?;
    Ok(threshold)
}

fn resolve_on_flagged_action(value: Option<&str>) -> OnFlaggedAction {
    match value {
        Some("BLOCK" | "block") => OnFlaggedAction::Block,
        Some("MONITOR" | "monitor") => OnFlaggedAction::Monitor,
        Some("PASSTHROUGH" | "passthrough") => OnFlaggedAction::Passthrough,
        _ => OnFlaggedAction::default(),
    }
}

fn resolve_reasoning_mode<L>(value: Option<String>, lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    if let Some(text) = value.filter(|text| matches!(text.as_str(), "off" | "hybrid" | "thinking"))
    {
        return Some(text);
    }
    lookup
        .lookup("GRAYSWAN_REASONING_MODE")
        .filter(|text| matches!(text.as_str(), "off" | "hybrid" | "thinking"))
}

