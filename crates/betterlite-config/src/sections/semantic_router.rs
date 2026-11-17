use std::path::PathBuf;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(rename = "auto_router_config_path")]
    config_path: Option<String>,
    #[serde(rename = "auto_router_config")]
    config_json: Option<String>,
    #[serde(rename = "auto_router_default_model")]
    default_model: Option<String>,
    #[serde(rename = "auto_router_embedding_model")]
    embedding_model: Option<String>,
    #[serde(rename = "auto_router_max_input_chars")]
    max_input_chars: Option<RawNumber>,
}

// every declared key stays optional: LiteLLM's router reports what is missing when it builds the strategy
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub config_path: Option<PathBuf>,
    pub config_json: Option<String>,
    pub default_model: Option<String>,
    pub embedding_model: Option<String>,
    pub max_input_chars: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures none of the five keys
    pub deployments: Vec<Option<Deployment>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(configured(deployment.litellm_params, index, lookup)?);
        }
        Ok(Settings { deployments })
    }
}

fn configured<L>(
    params: RawParams,
    index: usize,
    lookup: &L,
) -> Result<Option<Deployment>, ConfigError>
where
    L: EnvironmentLookup,
{
    if params.config_path.is_none()
        && params.config_json.is_none()
        && params.default_model.is_none()
        && params.embedding_model.is_none()
        && params.max_input_chars.is_none()
    {
        return Ok(None);
    }
    resolve_deployment(params, index, lookup).map(Some)
}

fn resolve_deployment<L>(
    params: RawParams,
    index: usize,
    lookup: &L,
) -> Result<Deployment, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("model_list.{index}.{PATH_ROOT}.{key}");
    let config_path =
        resolve_optional(params.config_path, &path("auto_router_config_path"), lookup)?
            .map(PathBuf::from);
    let config_json = resolve_optional(params.config_json, &path("auto_router_config"), lookup)?;
    let default_model = resolve_optional(
        params.default_model,
        &path("auto_router_default_model"),
        lookup,
    )?;
    let embedding_model = resolve_optional(
        params.embedding_model,
        &path("auto_router_embedding_model"),
        lookup,
    )?;
    Ok(Deployment {
        config_path,
        config_json,
        default_model,
        embedding_model,
        max_input_chars: resolve_limit(params.max_input_chars, index, lookup)?,
    })
}

fn resolve_limit(
    value: Option<RawNumber>,
    index: usize,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<i64>, ConfigError> {
    resolve_number(
        value,
        &format!("model_list.{index}.{PATH_ROOT}.auto_router_max_input_chars"),
        lookup,
        parse_input_chars,
    )
}

fn resolve_optional<L>(
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

fn parse_input_chars(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
