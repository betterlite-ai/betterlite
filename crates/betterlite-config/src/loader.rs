use std::collections::{BTreeMap, BTreeSet};
use std::num::{ParseFloatError, ParseIntError};

use secrecy::SecretString;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::providers::bedrock::request_policy::metadata_fields;
use crate::raw::{
    RawConfig, RawDeployment, RawGeneral, RawInfo, RawLiteLlm, RawNumber, RawParams, RawRouter,
};
use crate::resolve::{
    parse_float, parse_integer, resolve_number as number, resolve_secret, resolve_string,
};
use crate::secret_codec::general_settings;
use crate::sections::{RawSections, Sections};
use crate::types::{
    Config, GeneralSettings, LiteLlmParams, LiteLlmSettings, ModelDeployment, ModelInfo,
    ProxyTrust, RouterSettings,
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
    pub lookup: L,
}

impl<L> ConfigLoader<L>
where
    L: EnvironmentLookup,
{
    pub fn load(&self, source: &str) -> Result<Config, ConfigError> {
        let (raw, core_ignored) = parse_view::<RawConfig>(source)?;
        let mut ignored = vec![core_ignored];
        let raw_sections = RawSections::parse(source, &mut ignored)?;
        let paths = unknown_paths(&ignored);
        if !paths.is_empty() {
            return Err(ConfigError::Unsupported { paths });
        }
        let sections = raw_sections.convert(&self.lookup)?;
        convert(raw, sections, &self.lookup)
    }
}

pub(crate) fn parse_view<View>(source: &str) -> Result<(View, BTreeSet<String>), ConfigError>
where
    View: DeserializeOwned,
{
    let mut ignored = BTreeSet::new();
    let value = serde_saphyr::with_deserializer_from_str(source, |deserializer| {
        serde_ignored::deserialize(deserializer, |path| {
            ignored.insert(path.to_string());
        })
    })
    .map_err(|yaml_error| ConfigError::Yaml { source: yaml_error })?;
    Ok((value, ignored))
}

// a key is unknown when every view ignored it or one of its parents; only the outermost such paths are reported
pub(crate) fn unknown_paths(views: &[BTreeSet<String>]) -> Vec<String> {
    let mut unknown = BTreeSet::new();
    for path in views.iter().flatten() {
        let prefixes: Vec<&str> = path
            .match_indices('.')
            .map(|(index, _)| path.get(..index).unwrap_or(path))
            .chain([path.as_str()])
            .collect();
        if views
            .iter()
            .all(|view| prefixes.iter().any(|prefix| view.contains(*prefix)))
        {
            unknown.insert(path.as_str());
        }
    }
    unknown
        .iter()
        .filter(|path| {
            !unknown
                .iter()
                .any(|other| path.len() > other.len() && path.starts_with(&format!("{other}.")))
        })
        .map(|path| (*path).to_owned())
        .collect()
}

fn convert<L>(raw: RawConfig, sections: Sections, lookup: &L) -> Result<Config, ConfigError>
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
        sections,
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
    let optional = |value: Option<String>, path: &str| {
        value
            .map(|text| resolve_string(text, path, lookup))
            .transpose()
    };
    let model = optional(raw.model, &format!("{prefix}.model"))?;
    let api_base = optional(raw.api_base, &format!("{prefix}.api_base"))?;
    let api_key = resolve_secret(raw.api_key, &format!("{prefix}.api_key"), lookup)?;
    let api_version = optional(raw.api_version, &format!("{prefix}.api_version"))?;
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
    let id = raw
        .id
        .map(|text| resolve_string(text, &format!("model_list.{index}.model_info.id"), lookup))
        .transpose()?;
    Ok(ModelInfo { id })
}

fn convert_general<L>(raw: RawGeneral, lookup: &L) -> Result<GeneralSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let master_key = resolve_secret(raw.master_key, "general_settings.master_key", lookup)?;
    Ok(general_settings(
        raw.encryption_algorithm,
        master_key,
        lookup,
    ))
}

fn convert_router<L>(raw: RawRouter, lookup: &L) -> Result<RouterSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let integer = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_integer);
    let float = |value: Option<RawNumber>, path: &str| number(value, path, lookup, parse_float);
    let num_retries = integer(raw.num_retries, "router_settings.num_retries")?;
    let timeout = float(raw.timeout, "router_settings.timeout")?;
    let allowed_fails = integer(raw.allowed_fails, "router_settings.allowed_fails")?;
    let cooldown_time = float(raw.cooldown_time, "router_settings.cooldown_time")?;
    Ok(RouterSettings {
        routing_strategy: raw.routing_strategy,
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
    let retries_path = "litellm_settings.num_retries";
    let timeout_path = "litellm_settings.request_timeout";
    let num_retries = number(raw.num_retries, retries_path, lookup, parse_integer)?;
    let request_timeout = number(raw.request_timeout, timeout_path, lookup, parse_float)?;
    let (modify_params, default_max_tokens) = environment_defaults(raw.modify_params, lookup)?;
    Ok(LiteLlmSettings {
        num_retries,
        request_timeout,
        drop_params: raw.drop_params,
        proxy_trust: ProxyTrust {
            aiohttp_trust_env: raw.aiohttp_trust_env,
            disable_aiohttp_trust_env: raw.disable_aiohttp_trust_env,
        },
        bedrock_metadata_fields: metadata_fields(raw.bedrock_metadata_fields),
        modify_params,
        default_max_tokens,
        cost_discount_config: raw.cost_discount_config,
        cost_margin_config: raw.cost_margin_config,
    })
}

fn environment_defaults<L>(configured: Option<bool>, lookup: &L) -> Result<(bool, i64), ConfigError>
where
    L: EnvironmentLookup,
{
    // LiteLLM reads bool(os.getenv(...)), so every nonempty value, "false" included, turns it on
    let modify_params = configured.unwrap_or_else(|| {
        lookup
            .lookup("LITELLM_MODIFY_PARAMS")
            .is_some_and(|value| !value.is_empty())
    });
    let default_max_tokens = lookup
        .lookup("DEFAULT_MAX_TOKENS")
        .map_or(Ok(4096), |text| {
            text.trim()
                .parse::<i64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: "DEFAULT_MAX_TOKENS".to_owned(),
                    source,
                })
        })?;
    Ok((modify_params, default_max_tokens))
}

fn convert_environment<L>(
    values: BTreeMap<String, Option<String>>,
    lookup: &L,
) -> Result<BTreeMap<String, Option<SecretString>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = values
        .into_iter()
        .map(|(key, value)| {
            let path = format!("environment_variables.{key}");
            resolve_secret(value, &path, lookup).map(|secret| (key, secret))
        })
        .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
    Ok(resolved)
}
