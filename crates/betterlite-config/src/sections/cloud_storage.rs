use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub(crate) model_list: Vec<RawDeployment>,
    #[serde(default)]
    pub(crate) litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawDeployment {
    #[serde(default)]
    pub(crate) litellm_params: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawSettings {
    pub(crate) allow_legacy_cloud_file_ids: Option<RawSetting>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RawSetting {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub deployment_legacy: Option<RawSetting>,
    pub global_legacy: Option<RawSetting>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployment = self
            .model_list
            .into_iter()
            .next()
            .and_then(|item| item.litellm_params.allow_legacy_cloud_file_ids);
        Ok(Settings {
            deployment_legacy: deployment,
            global_legacy: self.litellm_settings.allow_legacy_cloud_file_ids,
        })
    }
}

#[path = "cloud_storage/tests.rs"]
mod tests;
