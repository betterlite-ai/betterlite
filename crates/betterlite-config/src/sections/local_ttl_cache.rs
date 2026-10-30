#[path = "local_ttl_cache/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
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

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub global_max_item_kib: i64,
    pub global_ttl: Option<f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            global_max_item_kib: 1024,
            global_ttl: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let global_max_item_kib = lookup
            .lookup("MAX_SIZE_PER_ITEM_IN_MEMORY_CACHE_IN_KB")
            .map(|text| {
                text.parse::<i64>()
                    .map_err(|source| ConfigError::InvalidInteger {
                        path: "MAX_SIZE_PER_ITEM_IN_MEMORY_CACHE_IN_KB".to_owned(),
                        source,
                    })
            })
            .transpose()?
            .unwrap_or(1024);
        let path = "litellm_settings.default_in_memory_ttl";
        let text = setting_text(
            self.litellm_settings.default_in_memory_ttl.as_ref(),
            path,
            lookup,
        )?;
        let global_ttl = text
            .map(|value| {
                value
                    .parse::<f64>()
                    .map_err(|source| ConfigError::InvalidFloat {
                        path: path.to_owned(),
                        source,
                    })
            })
            .transpose()?;
        Ok(Settings {
            global_max_item_kib,
            global_ttl,
        })
    }
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
