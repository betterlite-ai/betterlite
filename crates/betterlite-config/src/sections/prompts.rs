use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

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

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(prompts) = self.prompts else {
            return Ok(Settings::default());
        };
        match prompts {
            Json::Null => Ok(Settings::default()),
            Json::List(entries) => {
                let mut configured = Vec::with_capacity(entries.len());
                for (index, entry) in entries.into_iter().enumerate() {
                    let Json::Object(fields) = entry else {
                        return Err(ConfigError::InvalidScalar {
                            path: format!("prompts[{index}]"),
                            value: false,
                        });
                    };
                    configured.push(fields);
                }
                Ok(Settings {
                    prompts: configured,
                })
            }
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Err(ConfigError::InvalidScalar {
                    path: "prompts".to_owned(),
                    value: false,
                })
            }
        }
    }
}
