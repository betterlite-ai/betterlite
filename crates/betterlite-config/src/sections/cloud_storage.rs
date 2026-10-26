use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::{Deserialize, de::IgnoredAny};
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
    allow_legacy_cloud_file_ids: Option<RawSetting>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RawSetting {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub deployment_legacy: Vec<RawSetting>,
    pub global_legacy: Option<RawSetting>,
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            deployment_legacy: self
                .model_list
                .into_iter()
                .map(|item| {
                    item.litellm_params
                        .allow_legacy_cloud_file_ids
                        .unwrap_or(RawSetting::Boolean(false))
                })
                .collect(),
            global_legacy: self.litellm_settings.allow_legacy_cloud_file_ids,
        })
    }
}
#[path = "cloud_storage/tests.rs"]
mod tests;
