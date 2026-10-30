#[path = "local_ttl_cache/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(rename = "max_size_per_item_in_memory_cache_in_kb")]
    max_item_kib: Option<RawSetting>,
    default_in_memory_ttl: Option<RawSetting>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSetting {
    Integer(i64),
    Float(f64),
    Text(String),
    Boolean(bool),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub deployment_max_item_kib: Vec<Option<i64>>,
    pub global_max_item_kib: Option<i64>,
    pub deployment_ttl: Vec<Option<f64>>,
    pub global_ttl: Option<f64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployment_max = self
            .model_list
            .iter()
            .map(|deployment| {
                parse_optional_integer(
                    &deployment.litellm_params.max_item_kib,
                    "model_list.0.litellm_params.max_size_per_item_in_memory_cache_in_kb",
                    lookup,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let deployment_ttl = self
            .model_list
            .iter()
            .map(|deployment| {
                parse_optional_float(
                    &deployment.litellm_params.default_in_memory_ttl,
                    "model_list.0.litellm_params.default_in_memory_ttl",
                    lookup,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Settings {
            deployment_max_item_kib: deployment_max,
            global_max_item_kib: parse_optional_integer(
                &self.litellm_settings.max_item_kib,
                "litellm_settings.max_size_per_item_in_memory_cache_in_kb",
                lookup,
            )?
            .or(Some(1024)),
            deployment_ttl,
            global_ttl: parse_optional_float(
                &self.litellm_settings.default_in_memory_ttl,
                "litellm_settings.default_in_memory_ttl",
                lookup,
            )?,
        })
    }
}

fn parse_optional_integer<L>(
    value: &Option<RawSetting>,
    path: &str,
    lookup: &L,
) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let parsed = value
        .as_ref()
        .map(|value| parse_integer(value, path, lookup))
        .transpose();
    parsed
}

fn parse_optional_float<L>(
    value: &Option<RawSetting>,
    path: &str,
    lookup: &L,
) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let parsed = value
        .as_ref()
        .map(|value| parse_float(value, path, lookup))
        .transpose();
    parsed
}

fn text<L>(value: &RawSetting, path: &str, lookup: &L) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        RawSetting::Integer(value) => Ok(value.to_string()),
        RawSetting::Float(value) => Ok(value.to_string()),
        RawSetting::Text(value) => resolve_string(value.clone(), path, lookup),
        RawSetting::Boolean(value) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: *value,
        }),
    }
}

fn parse_integer<L>(value: &RawSetting, path: &str, lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    let text = text(value, path, lookup)?;
    let parsed = text
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        });
    parsed
}

fn parse_float<L>(value: &RawSetting, path: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let text = text(value, path, lookup)?;
    let parsed = text
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: path.to_owned(),
            source,
        });
    parsed
}
