use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    drop_params: Option<bool>,
    additional_drop_params: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    drop_params: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentDropPolicy {
    pub drop_params: Option<bool>,
    pub additional_drop_params: Vec<String>,
}

// deployments stay index-aligned with model_list, which the router keeps in config order
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub global_drop_params: bool,
    pub deployments: Vec<DeploymentDropPolicy>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment_text = lookup.lookup("LITELLM_DROP_PARAMS").unwrap_or_default();
        let environment = betterlite_env::flag("LITELLM_DROP_PARAMS", &environment_text)?;
        Ok(Settings {
            global_drop_params: self
                .litellm_settings
                .drop_params
                .or(environment)
                .unwrap_or(false),
            deployments: self
                .model_list
                .into_iter()
                .map(|deployment| DeploymentDropPolicy {
                    drop_params: deployment.litellm_params.drop_params,
                    additional_drop_params: deployment
                        .litellm_params
                        .additional_drop_params
                        .unwrap_or_default(),
                })
                .collect(),
        })
    }
}
