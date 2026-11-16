use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_secret, resolve_string};
use crate::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Knobs {
    pub(super) target_compression_ratio: Option<RawNumber>,
    pub(super) coarse: Option<RawFlag>,
    pub(super) min_chars_to_compress: Option<RawNumber>,
    pub(super) compress_tool_outputs: Option<RawFlag>,
    pub(super) compress_system: Option<RawFlag>,
    pub(super) compress_history: Option<RawFlag>,
    pub(super) compress_last_user: Option<RawFlag>,
    pub(super) enable_retrieval: Option<RawFlag>,
    pub(super) max_bytes_per_call: Option<RawNumber>,
    pub(super) allow_bypass_header: Option<RawFlag>,
    pub(super) dynamic: Option<RawFlag>,
    pub(super) dynamic_min_ratio: Option<RawNumber>,
    pub(super) dynamic_max_ratio: Option<RawNumber>,
    pub(super) compression_params: Option<BTreeMap<String, GuardrailValue>>,
}

// A knob left out stays None here, and the guardrail's constructor applies LiteLLM's default
#[derive(Debug, Clone, Default)]
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
    pub(super) fn float(
        &self,
        raw: Option<RawNumber>,
        name: &str,
    ) -> Result<Option<f64>, ConfigError> {
        let path = format!("{}.{name}", self.prefix);
        resolve_number(raw, &path, self.lookup, parse_float)
    }

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

    pub(super) fn secret(
        &self,
        raw: Option<SecretString>,
        name: &str,
    ) -> Result<Option<SecretString>, ConfigError> {
        let path = format!("{}.{name}", self.prefix);
        resolve_secret(raw, &path, self.lookup)
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

impl Knobs {
    pub(super) fn resolve<L>(self, resolver: &Resolver<'_, L>) -> Result<Tuning, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Tuning {
            target_compression_ratio: resolver
                .float(self.target_compression_ratio, "target_compression_ratio")?,
            coarse: resolver.flag(self.coarse, "coarse")?,
            min_chars_to_compress: resolver
                .integer(self.min_chars_to_compress, "min_chars_to_compress")?,
            compress_tool_outputs: resolver
                .flag(self.compress_tool_outputs, "compress_tool_outputs")?,
            compress_system: resolver.flag(self.compress_system, "compress_system")?,
            compress_history: resolver.flag(self.compress_history, "compress_history")?,
            compress_last_user: resolver.flag(self.compress_last_user, "compress_last_user")?,
            enable_retrieval: resolver.flag(self.enable_retrieval, "enable_retrieval")?,
            max_bytes_per_call: resolver.integer(self.max_bytes_per_call, "max_bytes_per_call")?,
            allow_bypass_header: resolver.flag(self.allow_bypass_header, "allow_bypass_header")?,
            dynamic: resolver.flag(self.dynamic, "dynamic")?,
            dynamic_min_ratio: resolver.float(self.dynamic_min_ratio, "dynamic_min_ratio")?,
            dynamic_max_ratio: resolver.float(self.dynamic_max_ratio, "dynamic_max_ratio")?,
            compression_params: self.compression_params,
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
