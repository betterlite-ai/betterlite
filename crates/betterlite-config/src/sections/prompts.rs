use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use self::environment::resolve_entry;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod environment;
mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    prompts: Option<Json>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub prompts: Vec<IndexMap<String, Json>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(prompts) = self.prompts else {
            return Ok(Settings::default());
        };
        let entries = match prompts {
            Json::Null => return Ok(Settings::default()),
            Json::List(entries) => entries,
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                return Err(ConfigError::InvalidScalar {
                    path: "prompts".to_owned(),
                    value: false,
                });
            }
        };
        let mut configured = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            let Json::Object(fields) = entry else {
                return Err(ConfigError::InvalidScalar {
                    path: format!("prompts[{index}]"),
                    value: false,
                });
            };
            configured.push(resolve_entry(fields, lookup)?);
        }
        Ok(Settings {
            prompts: configured,
        })
    }
}
