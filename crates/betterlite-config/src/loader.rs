use std::collections::{BTreeMap, BTreeSet};
use std::num::{ParseFloatError, ParseIntError};

use thiserror::Error;

use crate::raw::{
    RawConfig, RawDeployment, RawGeneral, RawInfo, RawLiteLlm, RawNumber, RawParams, RawRouter,
};
use crate::resolve::{
    parse_float, parse_integer, resolve_number as number, resolve_optional, resolve_secret,
    resolve_string,
};
use crate::types::{
    Config, GeneralSettings, LiteLlmParams, LiteLlmSettings, ModelDeployment, ModelInfo,
    RouterSettings,
};

pub trait EnvironmentLookup {
    fn lookup(&self, name: &str) -> Option<String>;
}

impl EnvironmentLookup for BTreeMap<String, String> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid YAML: {source}")]
    Yaml {
        #[source]
        source: serde_saphyr::Error,
    },
    #[error("unsupported config keys: {paths:?}")]
    Unsupported { paths: Vec<String> },
    #[error("environment variable {variable} for {path} is not set")]
    MissingEnvironment { variable: String, path: String },
    #[error("invalid number at {path}: {source}")]
    InvalidFloat {
        path: String,
        #[source]
        source: ParseFloatError,
    },
    #[error("invalid integer at {path}: {source}")]
    InvalidInteger {
        path: String,
        #[source]
        source: ParseIntError,
    },
    #[error("invalid scalar at {path}")]
    InvalidScalar { path: String, value: bool },
}

pub struct ConfigLoader<L> {
    lookup: L,
}

impl<L> ConfigLoader<L>
where
    L: EnvironmentLookup,
{
    pub const fn new(lookup: L) -> Self {
        let value = lookup;
        Self { lookup: value }
    }

    pub fn load(&self, source: &str) -> Result<Config, ConfigError> {
        let raw = parse(source)?;
        if !raw.ignored.is_empty() {
            return Err(ConfigError::Unsupported { paths: raw.ignored });
        }
        convert(raw.value, &self.lookup)
    }
}

struct ParsedConfig {
    value: RawConfig,
    ignored: Vec<String>,
}

fn parse(source: &str) -> Result<ParsedConfig, ConfigError> {
    let mut ignored = BTreeSet::new();
    let value = serde_saphyr::with_deserializer_from_str(source, |deserializer| {
        serde_ignored::deserialize(deserializer, |path| {
            ignored.insert(path.to_string());
        })
    })
    .map_err(|yaml_error| ConfigError::Yaml { source: yaml_error })?;
    Ok(ParsedConfig {
        value,
        ignored: ignored.into_iter().collect(),
    })
}

fn convert<L>(raw: RawConfig, lookup: &L) -> Result<Config, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut model_list = Vec::with_capacity(raw.model_list.len());
    for (index, deployment) in raw.model_list.into_iter().enumerate() {
        model_list.push(convert_deployment(deployment, index, lookup)?);
    }
    Ok(Config {
        model_list,
        general_settings: convert_general(raw.general_settings, lookup)?,
        router_settings: convert_router(raw.router_settings, lookup)?,
        litellm_settings: convert_litellm(raw.litellm_settings, lookup)?,
        environment_variables: convert_environment(raw.environment_variables, lookup)?,
    })
}

fn convert_deployment<L>(
    raw: RawDeployment,
    index: usize,
    lookup: &L,
) -> Result<ModelDeployment, ConfigError>
where
    L: EnvironmentLookup,
{
    let litellm_params = convert_params(raw.litellm_params, index, lookup)?;
    let model_info = convert_info(raw.model_info, index, lookup)?;
    let model_name = resolve_string(
        raw.model_name,
        &format!("model_list.{index}.model_name"),
        lookup,
    )?;
    Ok(ModelDeployment {
        model_name,
        litellm_params,
        model_info,
    })
}

fn convert_params<L>(raw: RawParams, index: usize, lookup: &L) -> Result<LiteLlmParams, ConfigError>
where
    L: EnvironmentLookup,
{
    let prefix = format!("model_list.{index}.litellm_params");
    let model = resolve_optional(raw.model, &format!("{prefix}.model"), lookup)?;
    let api_base = resolve_optional(raw.api_base, &format!("{prefix}.api_base"), lookup)?;
    let api_key = resolve_secret(raw.api_key, &format!("{prefix}.api_key"), lookup)?;
    let api_version = resolve_optional(raw.api_version, &format!("{prefix}.api_version"), lookup)?;
    let float = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_float);
    let integer = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_integer);
    let timeout = float(raw.timeout, &format!("{prefix}.timeout"))?;
    let stream_timeout = float(raw.stream_timeout, &format!("{prefix}.stream_timeout"))?;
    let max_retries = integer(raw.max_retries, &format!("{prefix}.max_retries"))?;
    let rpm = integer(raw.rpm, &format!("{prefix}.rpm"))?;
    let tpm = integer(raw.tpm, &format!("{prefix}.tpm"))?;
    let weight = integer(raw.weight, &format!("{prefix}.weight"))?;
    Ok(LiteLlmParams {
        model,
        api_base,
        api_key,
        api_version,
        timeout,
        stream_timeout,
        max_retries,
        rpm,
        tpm,
        weight,
    })
}

fn convert_info<L>(raw: RawInfo, index: usize, lookup: &L) -> Result<ModelInfo, ConfigError>
where
    L: EnvironmentLookup,
{
    let id = resolve_optional(raw.id, &format!("model_list.{index}.model_info.id"), lookup)?;
    Ok(ModelInfo { id })
}

fn convert_general<L>(raw: RawGeneral, lookup: &L) -> Result<GeneralSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let master_key = resolve_secret(raw.master_key, "general_settings.master_key", lookup)?;
    let settings = GeneralSettings { master_key };
    Ok(settings)
}

fn convert_router<L>(raw: RawRouter, lookup: &L) -> Result<RouterSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let routing_strategy = raw.routing_strategy;
    let integer = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_integer);
    let float = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_float);
    let num_retries = integer(raw.num_retries, "router_settings.num_retries")?;
    let timeout = float(raw.timeout, "router_settings.timeout")?;
    let allowed_fails = integer(raw.allowed_fails, "router_settings.allowed_fails")?;
    let cooldown_time = float(raw.cooldown_time, "router_settings.cooldown_time")?;
    Ok(RouterSettings {
        routing_strategy,
        num_retries,
        timeout,
        allowed_fails,
        cooldown_time,
        fallbacks: raw.fallbacks,
    })
}

fn convert_litellm<L>(raw: RawLiteLlm, lookup: &L) -> Result<LiteLlmSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let settings_path = "litellm_settings";
    let retries_path = format!("{settings_path}.num_retries");
    let timeout_path = format!("{settings_path}.request_timeout");
    let num_retries = number(raw.num_retries, &retries_path, lookup, parse_integer)?;
    let request_timeout = number(raw.request_timeout, &timeout_path, lookup, parse_float)?;
    let drop_params = raw.drop_params;
    Ok(LiteLlmSettings {
        num_retries,
        request_timeout,
        drop_params,
    })
}

fn convert_environment<L>(
    values: BTreeMap<String, Option<String>>,
    lookup: &L,
) -> Result<BTreeMap<String, Option<String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = BTreeMap::new();
    for (key, value) in values {
        let path = format!("environment_variables.{key}");
        resolved.insert(key, resolve_optional(value, &path, lookup)?);
    }
    Ok(resolved)
}
