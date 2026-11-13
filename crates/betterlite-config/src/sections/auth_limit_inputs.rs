use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// One deployment's declared per-key defaults, kept raw so the auth projection applies int()'s skip and overflow rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeploymentLimitDefaults {
    pub model_name: String,
    pub default_api_key_rpm_limit: Option<Json>,
    pub default_api_key_tpm_limit: Option<Json>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub deployments: Vec<DeploymentLimitDefaults>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    model_name: String,
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(default)]
    default_api_key_rpm_limit: Option<Json>,
    #[serde(default)]
    default_api_key_tpm_limit: Option<Json>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .map(|deployment| DeploymentLimitDefaults {
                model_name: deployment.model_name,
                default_api_key_rpm_limit: deployment.litellm_params.default_api_key_rpm_limit,
                default_api_key_tpm_limit: deployment.litellm_params.default_api_key_tpm_limit,
            })
            .collect();
        Ok(Settings { deployments })
    }
}
