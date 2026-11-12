use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::model_capability_predicates::CapabilityRequest;
use serde_json::Value;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::openai_base_config::{CatalogCapabilities, ModelFamily, Object, model_family};

pub mod audio;
pub mod gpt_5;
pub mod o_series;
mod tests;

const DEFAULT_EFFORT_KEY: &str = "default_reasoning_effort";
const GPT_5: &str = "gpt-5";
const SEARCH: &str = "search";

// The composing layer implements these with B2221's CapabilityPredicates, one call per method
pub trait ReasoningCapabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send;

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send;

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send;

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send;

    fn catalogue_declares_default(&mut self) -> bool;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropParams {
    pub global: bool,
    pub request: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedModel {
    pub model: String,
    pub provider: String,
}

// B174's resolver, so the leaf never imports betterlite-provider back
pub trait ProviderResolution {
    fn resolve_provider(&mut self, model: &str) -> Option<ResolvedModel>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningPolicy {
    OSeries,
    Gpt5Search,
    Gpt5,
    Audio,
    Base,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReasoningError {
    #[error("reasoning_effort={effort} is not supported for this model")]
    UnsupportedEffort { effort: String },
    #[error("{model} does not support temperature={temperature} while reasoning is active")]
    UnsupportedTemperature { model: String, temperature: Value },
    #[error(
        "{model} only supports logprobs, top_p and top_logprobs when reasoning_effort resolves to none"
    )]
    UnsupportedSampling {
        model: String,
        effort: Option<String>,
    },
}

impl ReasoningError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedEffort { .. }
            | Self::UnsupportedTemperature { .. }
            | Self::UnsupportedSampling { .. } => 400,
        }
    }
}

#[must_use]
pub fn is_model_gpt_5_search_model(model: &str) -> bool {
    if model.contains(GPT_5) {
        model.contains(SEARCH)
    } else {
        false
    }
}

#[must_use]
pub fn gpt_series_version(model: &str) -> Option<(u64, u64)> {
    let name = model.rsplit('/').next().unwrap_or(model);
    let rest = name.strip_prefix("gpt-")?;
    let major_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if major_len == 0 {
        return None;
    }
    let (major_text, after_major) = rest.split_at(major_len);
    let major = major_text.parse::<u64>().ok()?;
    let Some(after_dot) = after_major.strip_prefix('.') else {
        if after_major.is_empty() || after_major.starts_with('-') {
            return Some((major, 0));
        }
        return None;
    };
    let minor_len = after_dot.bytes().take_while(u8::is_ascii_digit).count();
    if minor_len == 0 {
        return Some((major, 0));
    }
    let (minor_text, remainder) = after_dot.split_at(minor_len);
    if remainder.is_empty() || remainder.starts_with('.') || remainder.starts_with('-') {
        let minor = minor_text.parse::<u64>().ok()?;
        return Some((major, minor));
    }
    // a failed minor terminator backtracks to the major, whose '.' still satisfies the lookahead
    Some((major, 0))
}

#[must_use]
pub fn is_model_gpt_5_2_model(model: &str) -> bool {
    let name = model.rsplit('/').next().unwrap_or(model);
    if name.starts_with("gpt-5.2") {
        return true;
    }
    name.starts_with("gpt-5.4")
}

#[must_use]
pub fn is_model_gpt_5_4_plus_model(model: &str) -> bool {
    let Some(version) = gpt_series_version(model) else {
        return false;
    };
    version >= (5, 4)
}

#[must_use]
pub fn is_model_gpt_5_6_plus_model(model: &str) -> bool {
    let Some(version) = gpt_series_version(model) else {
        return false;
    };
    version >= (5, 6)
}

#[must_use]
pub fn is_model_gpt_6_plus_model(model: &str) -> bool {
    let Some(version) = gpt_series_version(model) else {
        return false;
    };
    version >= (6, 0)
}

#[must_use]
pub fn effort_level(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::String(text) => Some(text),
        Value::Object(fields) => fields.get("effort").and_then(Value::as_str),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    }
}

pub(crate) fn is_temperature_one(value: &Value) -> bool {
    if let Some(number) = value.as_f64() {
        return number.to_bits() == 1.0_f64.to_bits();
    }
    false
}

pub(crate) fn copy_supported(non_default: &Object, optional: &mut Object, supported: &[&str]) {
    for (name, value) in non_default {
        if supported.contains(&name.as_str()) {
            optional.insert(name.clone(), value.clone());
        }
    }
}

#[must_use]
pub fn catalogue_declares_default<'catalogue, Entries>(entries: Entries) -> bool
where
    Entries: IntoIterator<Item = &'catalogue str>,
{
    entries.into_iter().any(|entry| {
        serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(entry)
            .is_ok_and(|fields| fields.contains_key(DEFAULT_EFFORT_KEY))
    })
}

#[must_use]
pub fn select_policy(model: &str, catalog: &CatalogCapabilities) -> ReasoningPolicy {
    match model_family(model, catalog) {
        ModelFamily::OSeries => ReasoningPolicy::OSeries,
        ModelFamily::GptReasoning => {
            if is_model_gpt_5_search_model(model) {
                ReasoningPolicy::Gpt5Search
            } else {
                ReasoningPolicy::Gpt5
            }
        }
        ModelFamily::Audio => ReasoningPolicy::Audio,
        ModelFamily::Gpt | ModelFamily::Unknown => ReasoningPolicy::Base,
    }
}

pub async fn effort_resolves_to_none<C>(
    capabilities: &mut C,
    model: &str,
    effective_effort: Option<&str>,
) -> bool
where
    C: ReasoningCapabilities,
{
    if let Some(effort) = effective_effort {
        return effort == "none";
    }
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    if let Some(declared) = capabilities
        .declared_value_factory(&request, DEFAULT_EFFORT_KEY)
        .await
    {
        return declared == "none";
    }
    if capabilities.catalogue_declares_default() {
        return false;
    }
    capabilities
        .supports_factory(&request, "supports_none_reasoning_effort")
        .await
}
