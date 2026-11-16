use std::collections::BTreeMap;

use serde::Deserialize;

use super::Parameters;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawFlag {
    Boolean(bool),
    Text(String),
}

// A knob left out stays None here, and the guardrail's constructor applies LiteLLM's default
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Tuning {
    pub target_compression_ratio: Option<f64>,
    pub coarse: Option<bool>,
    pub min_chars_to_compress: Option<i64>,
    pub compress_tool_outputs: Option<bool>,
    pub compress_system: Option<bool>,
    pub compress_history: Option<bool>,
    pub compress_last_user: Option<bool>,
    pub enable_retrieval: Option<bool>,
    pub max_bytes_per_call: Option<i64>,
    pub allow_bypass_header: Option<bool>,
    pub dynamic: Option<bool>,
    pub dynamic_min_ratio: Option<f64>,
    pub dynamic_max_ratio: Option<f64>,
    pub compression_params: Option<BTreeMap<String, GuardrailValue>>,
}

pub(super) struct Resolver<'config, L> {
    pub(super) prefix: &'config str,
    pub(super) lookup: &'config L,
}

impl<L> Resolver<'_, L>
where
    L: EnvironmentLookup,
{
    fn integer(&self, raw: Option<RawNumber>, name: &str) -> Result<Option<i64>, ConfigError> {
        let path = format!("{}.{name}", self.prefix);
        resolve_number(raw, &path, self.lookup, |text, at| {
            text.parse::<i64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: at.to_owned(),
                    source,
                })
        })
    }

    fn flag(&self, raw: Option<RawFlag>, name: &str) -> Result<Option<bool>, ConfigError> {
        let path = format!("{}.{name}", self.prefix);
        let text = match raw {
            None => return Ok(None),
            Some(RawFlag::Boolean(flag)) => return Ok(Some(flag)),
            Some(RawFlag::Text(text)) => text,
        };
        let variable = text.strip_prefix("os.environ/").unwrap_or(&path).to_owned();
        let resolved = resolve_string(text, &path, self.lookup)?;
        Ok(betterlite_env::flag(&variable, &resolved)?)
    }

    // An environment reference that resolves to an empty value counts as not set
    pub(super) fn text(
        &self,
        raw: Option<String>,
        name: &str,
    ) -> Result<Option<String>, ConfigError> {
        let Some(text) = raw else {
            return Ok(None);
        };
        let path = format!("{}.{name}", self.prefix);
        let is_reference = text.starts_with("os.environ/");
        let resolved = resolve_string(text, &path, self.lookup)?;
        if resolved.is_empty() && is_reference {
            return Ok(None);
        }
        Ok(Some(resolved))
    }
}

impl Parameters {
    // Takes the knob fields out, so the rest of the params stay to be read
    pub(super) fn tuning<L>(&mut self, resolver: &Resolver<'_, L>) -> Result<Tuning, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Tuning {
            target_compression_ratio: resolve_number(
                self.target_compression_ratio.take(),
                &format!("{}.target_compression_ratio", resolver.prefix),
                resolver.lookup,
                parse_float,
            )?,
            coarse: resolver.flag(self.coarse.take(), "coarse")?,
            min_chars_to_compress: resolver
                .integer(self.min_chars_to_compress.take(), "min_chars_to_compress")?,
            compress_tool_outputs: resolver
                .flag(self.compress_tool_outputs.take(), "compress_tool_outputs")?,
            compress_system: resolver.flag(self.compress_system.take(), "compress_system")?,
            compress_history: resolver.flag(self.compress_history.take(), "compress_history")?,
            compress_last_user: resolver
                .flag(self.compress_last_user.take(), "compress_last_user")?,
            enable_retrieval: resolver.flag(self.enable_retrieval.take(), "enable_retrieval")?,
            max_bytes_per_call: resolver
                .integer(self.max_bytes_per_call.take(), "max_bytes_per_call")?,
            allow_bypass_header: resolver
                .flag(self.allow_bypass_header.take(), "allow_bypass_header")?,
            dynamic: resolver.flag(self.dynamic.take(), "dynamic")?,
            dynamic_min_ratio: resolve_number(
                self.dynamic_min_ratio.take(),
                &format!("{}.dynamic_min_ratio", resolver.prefix),
                resolver.lookup,
                parse_float,
            )?,
            dynamic_max_ratio: resolve_number(
                self.dynamic_max_ratio.take(),
                &format!("{}.dynamic_max_ratio", resolver.prefix),
                resolver.lookup,
                parse_float,
            )?,
            compression_params: self.compression_params.take(),
        })
    }
}

impl Tuning {
    // LiteLLM's _get_optional_value: the nested value wins whenever it is set
    pub(super) fn or(self, flat: Self) -> Self {
        Self {
            target_compression_ratio: self
                .target_compression_ratio
                .or(flat.target_compression_ratio),
            coarse: self.coarse.or(flat.coarse),
            min_chars_to_compress: self.min_chars_to_compress.or(flat.min_chars_to_compress),
            compress_tool_outputs: self.compress_tool_outputs.or(flat.compress_tool_outputs),
            compress_system: self.compress_system.or(flat.compress_system),
            compress_history: self.compress_history.or(flat.compress_history),
            compress_last_user: self.compress_last_user.or(flat.compress_last_user),
            enable_retrieval: self.enable_retrieval.or(flat.enable_retrieval),
            max_bytes_per_call: self.max_bytes_per_call.or(flat.max_bytes_per_call),
            allow_bypass_header: self.allow_bypass_header.or(flat.allow_bypass_header),
            dynamic: self.dynamic.or(flat.dynamic),
            dynamic_min_ratio: self.dynamic_min_ratio.or(flat.dynamic_min_ratio),
            dynamic_max_ratio: self.dynamic_max_ratio.or(flat.dynamic_max_ratio),
            compression_params: self.compression_params.or(flat.compression_params),
        }
    }
}
