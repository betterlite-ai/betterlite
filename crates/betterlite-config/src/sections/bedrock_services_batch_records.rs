use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<Deployment>,
    #[serde(default)]
    litellm_settings: GlobalSettings,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: DeploymentParams,
}

#[derive(Debug, Default, Deserialize)]
struct DeploymentParams {
    // an explicit null is the same as absent, as pre_call reads the global key
    #[serde(default)]
    model_alias_map: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
struct GlobalSettings {
    #[serde(default)]
    model_alias_map: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // litellm_settings.model_alias_map, then each deployment's map in file order over it
    pub model_alias_map: BTreeMap<String, String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut model_alias_map = self.litellm_settings.model_alias_map.unwrap_or_default();
        for deployment in self.model_list {
            model_alias_map.extend(
                deployment
                    .litellm_params
                    .model_alias_map
                    .unwrap_or_default(),
            );
        }
        Ok(Settings { model_alias_map })
    }
}
