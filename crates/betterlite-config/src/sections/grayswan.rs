use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};

mod tests;

pub const UI_FRIENDLY_NAME: &str = "Gray Swan Cygnal";
const DEFAULT_API_BASE: &str = "https://api.grayswan.ai";
const DEFAULT_GUARDRAIL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OnFlaggedAction {
    Block,
    #[default]
    Monitor,
    Passthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningMode {
    Off,
    Hybrid,
    Thinking,
}

impl ReasoningMode {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "hybrid" => Some(Self::Hybrid),
            "thinking" => Some(Self::Thinking),
            _ => None,
        }
    }
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
    violation_threshold: Option<RawNumber>,
    reasoning_mode: Option<String>,
    categories: Option<BTreeMap<String, String>>,
    policy_id: Option<String>,
    fail_open: Option<bool>,
    guardrail_timeout: Option<f64>,
    streaming_end_of_stream_only: Option<bool>,
    streaming_sampling_rate: Option<u32>,
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
    pub reasoning_mode: Option<ReasoningMode>,
    pub categories: Option<BTreeMap<String, String>>,
    pub policy_id: Option<String>,
    pub fail_open: bool,
    pub guardrail_timeout: Duration,
    pub streaming_end_of_stream_only: bool,
    pub streaming_sampling_rate: u32,
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
        let api_base = match parameters.api_base.filter(|value| !value.is_empty()) {
            Some(value) => resolve_string(value, &format!("{prefix}.api_base"), lookup)?,
            None => lookup
                .lookup("GRAYSWAN_API_BASE")
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        };
        let violation_threshold = resolve_number(
            parameters.violation_threshold,
            &format!("{prefix}.violation_threshold"),
            lookup,
            parse_float,
        )?
        .unwrap_or_else(|| {
            lookup
                .lookup("GRAYSWAN_VIOLATION_THRESHOLD")
                .and_then(|text| text.parse::<f64>().ok())
                .unwrap_or(DEFAULT_VIOLATION_THRESHOLD)
        });
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_key,
            api_base: api_base.trim_end_matches('/').to_owned(),
            on_flagged_action: resolve_on_flagged_action(parameters.on_flagged_action.as_deref()),
            violation_threshold,
            reasoning_mode: resolve_reasoning_mode(parameters.reasoning_mode.as_deref(), lookup),
            categories: parameters.categories,
            policy_id: parameters.policy_id,
            fail_open: parameters.fail_open.unwrap_or(true),
            guardrail_timeout: resolve_guardrail_timeout(parameters.guardrail_timeout),
            streaming_end_of_stream_only: parameters.streaming_end_of_stream_only.unwrap_or(false),
            streaming_sampling_rate: parameters.streaming_sampling_rate.unwrap_or(5),
        })
    }
}

fn resolve_guardrail_timeout(guardrail_timeout: Option<f64>) -> Duration {
    let Some(secs) = guardrail_timeout else {
        return DEFAULT_GUARDRAIL_TIMEOUT;
    };
    Duration::try_from_secs_f64(secs).unwrap_or_else(|source| {
        tracing::warn!(
            secs,
            error = %source,
            default = ?DEFAULT_GUARDRAIL_TIMEOUT,
            "Gray Swan Guardrail: guardrail_timeout cannot be represented as a duration, defaulting"
        );
        DEFAULT_GUARDRAIL_TIMEOUT
    })
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

fn resolve_on_flagged_action(value: Option<&str>) -> OnFlaggedAction {
    let Some(text) = value else {
        return OnFlaggedAction::default();
    };
    match text.to_ascii_lowercase().as_str() {
        "block" => OnFlaggedAction::Block,
        "monitor" => OnFlaggedAction::Monitor,
        "passthrough" => OnFlaggedAction::Passthrough,
        _ => {
            tracing::warn!(
                on_flagged_action = text,
                default = ?OnFlaggedAction::default(),
                "Gray Swan Guardrail: unsupported on_flagged_action, defaulting"
            );
            OnFlaggedAction::default()
        }
    }
}

fn resolve_reasoning_mode<L>(value: Option<&str>, lookup: &L) -> Option<ReasoningMode>
where
    L: EnvironmentLookup,
{
    if let Some(mode) = value.and_then(ReasoningMode::parse) {
        return Some(mode);
    }
    lookup
        .lookup("GRAYSWAN_REASONING_MODE")
        .as_deref()
        .and_then(ReasoningMode::parse)
}
