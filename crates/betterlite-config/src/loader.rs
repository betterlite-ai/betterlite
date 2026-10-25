use std::collections::{BTreeMap, BTreeSet};
use std::num::{ParseFloatError, ParseIntError};

use secrecy::SecretString;
use thiserror::Error;

use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::providers::bedrock::request_policy::metadata_fields;
use crate::raw::{
    RawConfig, RawDeployment, RawGeneral, RawInfo, RawLiteLlm, RawNumber, RawParams, RawRouter,
};
use crate::resolve::{
    parse_float, parse_integer, resolve_number as number, resolve_secret, resolve_string,
};
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
    Ok(GeneralSettings { master_key })
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

fn convert_litellm<L>(mut raw: RawLiteLlm, lookup: &L) -> Result<LiteLlmSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let retries_path = "litellm_settings.num_retries";
    let timeout_path = "litellm_settings.request_timeout";
    let resource_sdk_settings = resource_sdk_settings(&mut raw, lookup)?;
    let num_retries = number(raw.num_retries, retries_path, lookup, parse_integer)?;
    let request_timeout = number(raw.request_timeout, timeout_path, lookup, parse_float)?;
    let (modify_params, default_max_tokens) = environment_defaults(raw.modify_params, lookup)?;
    Ok(LiteLlmSettings {
        resource_sdk_settings,
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
    })
}

fn resource_sdk_settings<L>(
    raw: &mut RawLiteLlm,
    lookup: &L,
) -> Result<ResourceSdkSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut settings = resource_sdk_environment(lookup);
    settings.api_base = optional_string(raw.api_base.take(), "litellm_settings.api_base", lookup)?;
    settings.api_version = optional_string(
        raw.api_version.take(),
        "litellm_settings.api_version",
        lookup,
    )?;
    settings.api_key = resolve_secret(raw.api_key.take(), "litellm_settings.api_key", lookup)?;
    settings.openai_key =
        resolve_secret(raw.openai_key.take(), "litellm_settings.openai_key", lookup)?;
    settings.azure_key =
        resolve_secret(raw.azure_key.take(), "litellm_settings.azure_key", lookup)?;
    settings.azure_ad_token = resolve_secret(
        raw.azure_ad_token.take(),
        "litellm_settings.azure_ad_token",
        lookup,
    )?;
    settings.organization = optional_string(
        raw.organization.take(),
        "litellm_settings.organization",
        lookup,
    )?;
    settings.vertex_project = optional_string(
        raw.vertex_project.take(),
        "litellm_settings.vertex_project",
        lookup,
    )?;
    settings.vertex_location = optional_string(
        raw.vertex_location.take(),
        "litellm_settings.vertex_location",
        lookup,
    )?;
    settings.vertex_credentials = resolve_secret(
        raw.vertex_credentials.take(),
        "litellm_settings.vertex_credentials",
        lookup,
    )?;
    Ok(settings)
}

fn optional_string<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    value
        .map(|text| resolve_string(text, path, lookup))
        .transpose()
}

fn resource_sdk_environment<L>(lookup: &L) -> ResourceSdkSettings
where
    L: EnvironmentLookup,
{
    let environment = |name: &str| lookup.lookup(name);
    ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        openai_base_url: environment("OPENAI_BASE_URL"),
        openai_api_base: environment("OPENAI_API_BASE"),
        openai_api_key: environment("OPENAI_API_KEY").map(SecretString::from),
        openai_organization: environment("OPENAI_ORGANIZATION"),
        azure_api_base: environment("AZURE_API_BASE"),
        azure_api_version: environment("AZURE_API_VERSION"),
        azure_openai_api_key: environment("AZURE_OPENAI_API_KEY").map(SecretString::from),
        azure_api_key: environment("AZURE_API_KEY").map(SecretString::from),
        vertexai_project: environment("VERTEXAI_PROJECT"),
        vertexai_location: environment("VERTEXAI_LOCATION"),
        vertexai_credentials: environment("VERTEXAI_CREDENTIALS").map(SecretString::from),
    }
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
