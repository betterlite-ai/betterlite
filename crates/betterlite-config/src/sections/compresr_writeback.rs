use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;
use serde::de::Error as DeserializeError;

use self::knobs::{RawFlag, Resolver, Tuning};
use super::Section;
use super::guardrail_identity::item::GuardrailValue;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_secret;

pub mod knobs;
mod tests;

const GUARDRAIL: &str = "compresr";

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

// LiteLLM reads each tuning knob from optional_params first and from the params themselves second
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    model: Option<String>,
    unreachable_fallback: Option<String>,
    timeout: Option<RawNumber>,
    target_compression_ratio: Option<RawNumber>,
    coarse: Option<RawFlag>,
    min_chars_to_compress: Option<RawNumber>,
    compress_tool_outputs: Option<RawFlag>,
    compress_system: Option<RawFlag>,
    compress_history: Option<RawFlag>,
    compress_last_user: Option<RawFlag>,
    enable_retrieval: Option<RawFlag>,
    max_bytes_per_call: Option<RawNumber>,
    allow_bypass_header: Option<RawFlag>,
    dynamic: Option<RawFlag>,
    dynamic_min_ratio: Option<RawNumber>,
    dynamic_max_ratio: Option<RawNumber>,
    compression_params: Option<BTreeMap<String, GuardrailValue>>,
    optional_params: Option<Box<Self>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnreachableFallback {
    #[default]
    FailClosed,
    FailOpen,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: Option<f64>,
    pub tuning: Tuning,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == GUARDRAIL {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

impl UnreachableFallback {
    fn resolve<L>(raw: Option<String>, resolver: &Resolver<'_, L>) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        match resolver.text(raw, "unreachable_fallback")?.as_deref() {
            None | Some("fail_closed") => Ok(Self::FailClosed),
            Some("fail_open") => Ok(Self::FailOpen),
            Some(other) => Err(ConfigError::Yaml {
                source: <serde_saphyr::Error as DeserializeError>::custom(format!(
                    "{}.unreachable_fallback: invalid unreachable_fallback `{other}`; expected `fail_closed` or `fail_open`",
                    resolver.prefix
                )),
            }),
        }
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let resolver = Resolver {
            prefix: &prefix,
            lookup,
        };
        let mut parameters = self.litellm_params;
        let mut tuning = parameters.tuning(&resolver)?;
        if let Some(mut nested) = parameters.optional_params.take() {
            let nested_prefix = format!("{prefix}.optional_params");
            let nested_resolver = Resolver {
                prefix: &nested_prefix,
                lookup,
            };
            tuning = nested.tuning(&nested_resolver)?.or(tuning);
        }
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            api_base: resolver.text(parameters.api_base, "api_base")?,
            api_key: resolve_secret(parameters.api_key, &format!("{prefix}.api_key"), lookup)?,
            model: resolver.text(parameters.model, "model")?,
            unreachable_fallback: UnreachableFallback::resolve(
                parameters.unreachable_fallback,
                &resolver,
            )?,
            timeout: resolver.float(parameters.timeout, "timeout")?,
            tuning,
        })
    }
}
