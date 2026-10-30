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
        let deployment_max_item_kib = self
            .model_list
            .iter()
            .map(|deployment| {
                parse_max_item(
                    deployment.litellm_params.max_item_kib.as_ref(),
                    lookup,
                    "model_list.0.litellm_params.max_size_per_item_in_memory_cache_in_kb",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let deployment_ttl = self
            .model_list
            .iter()
            .map(|deployment| {
                parse_ttl(
                    deployment.litellm_params.default_in_memory_ttl.as_ref(),
                    lookup,
                    "model_list.0.litellm_params.default_in_memory_ttl",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let global_max_item_kib = parse_max_item(
            self.litellm_settings.max_item_kib.as_ref(),
            lookup,
            "litellm_settings.max_size_per_item_in_memory_cache_in_kb",
        )?
        .or(Some(1024));
        let global_ttl = parse_ttl(
            self.litellm_settings.default_in_memory_ttl.as_ref(),
            lookup,
            "litellm_settings.default_in_memory_ttl",
        )?;
        Ok(Settings {
            deployment_max_item_kib,
            global_max_item_kib,
            deployment_ttl,
            global_ttl,
        })
    }
}

fn parse_max_item<L>(
    raw: Option<&RawSetting>,
    lookup: &L,
    path: &str,
) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let parsed = parse_setting(raw, path, lookup, |text| {
        text.parse::<i64>()
            .map_err(|source| ConfigError::InvalidInteger {
                path: path.to_owned(),
                source,
            })
    })?;
    Ok(parsed)
}

fn parse_ttl<L>(
    raw: Option<&RawSetting>,
    lookup: &L,
    path: &str,
) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let parsed = parse_setting(raw, path, lookup, |text| {
        text.parse::<f64>()
            .map_err(|source| ConfigError::InvalidFloat {
                path: path.to_owned(),
                source,
            })
    })?;
    Ok(parsed)
}

fn parse_setting<T, L, F>(
    raw: Option<&RawSetting>,
    path: &str,
    lookup: &L,
    parse: F,
) -> Result<Option<T>, ConfigError>
where
    L: EnvironmentLookup,
    F: FnOnce(String) -> Result<T, ConfigError>,
{
    let text = setting_text(raw, path, lookup)?;
    text.map(parse).transpose()
}

fn setting_text<L>(
    raw: Option<&RawSetting>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw_setting) = raw else {
        return Ok(None);
    };
    let text = match raw_setting {
        RawSetting::Integer(integer) => integer.to_string(),
        RawSetting::Float(float) => float.to_string(),
        RawSetting::Text(text) => resolve_string(text.clone(), path, lookup)?,
        RawSetting::Boolean(boolean) => {
            return Err(ConfigError::InvalidScalar {
                path: path.to_owned(),
                value: *boolean,
            });
        }
    };
    Ok(Some(text))
}
