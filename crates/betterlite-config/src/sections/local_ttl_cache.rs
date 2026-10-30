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
        let deployment_max_item_kib = deployment_settings(
            &self.model_list,
            lookup,
            "model_list.0.litellm_params.max_size_per_item_in_memory_cache_in_kb",
            |settings| settings.max_item_kib.as_ref(),
            |text| {
                text.parse::<i64>()
                    .map_err(|source| ConfigError::InvalidInteger {
                        path: "model_list.0.litellm_params.max_size_per_item_in_memory_cache_in_kb"
                            .to_owned(),
                        source,
                    })
            },
        )?;
        let deployment_ttl = deployment_settings(
            &self.model_list,
            lookup,
            "model_list.0.litellm_params.default_in_memory_ttl",
            |settings| settings.default_in_memory_ttl.as_ref(),
            |text| {
                text.parse::<f64>()
                    .map_err(|source| ConfigError::InvalidFloat {
                        path: "model_list.0.litellm_params.default_in_memory_ttl".to_owned(),
                        source,
                    })
            },
        )?;
        let (global_max_item_kib, global_ttl) = global_settings(&self.litellm_settings, lookup)?;
        Ok(Settings {
            deployment_max_item_kib,
            global_max_item_kib,
            deployment_ttl,
            global_ttl,
        })
    }
}

fn global_settings<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<(Option<i64>, Option<f64>), ConfigError>
where
    L: EnvironmentLookup,
{
    let max_item_kib = parse_setting(
        settings.max_item_kib.as_ref(),
        "litellm_settings.max_size_per_item_in_memory_cache_in_kb",
        lookup,
        |text| {
            text.parse::<i64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: "litellm_settings.max_size_per_item_in_memory_cache_in_kb".to_owned(),
                    source,
                })
        },
    )?
    .or(Some(1024));
    let ttl = parse_setting(
        settings.default_in_memory_ttl.as_ref(),
        "litellm_settings.default_in_memory_ttl",
        lookup,
        |text| {
            text.parse::<f64>()
                .map_err(|source| ConfigError::InvalidFloat {
                    path: "litellm_settings.default_in_memory_ttl".to_owned(),
                    source,
                })
        },
    )?;
    Ok((max_item_kib, ttl))
}

fn deployment_settings<T, L, F, P>(
    deployments: &[RawDeployment],
    lookup: &L,
    path: &str,
    field: F,
    parse: P,
) -> Result<Vec<Option<T>>, ConfigError>
where
    L: EnvironmentLookup,
    F: Fn(&RawSettings) -> Option<&RawSetting>,
    P: Fn(String) -> Result<T, ConfigError> + Copy,
{
    let values = deployments
        .iter()
        .map(|deployment| parse_setting(field(&deployment.litellm_params), path, lookup, parse));
    values.collect()
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
