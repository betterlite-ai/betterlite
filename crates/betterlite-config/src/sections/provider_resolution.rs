use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(rename = "custom_llm_provider")]
    custom_llm_provider: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Vec<Option<String>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .enumerate()
            .map(|(index, deployment)| {
                let path = format!("model_list.{index}.litellm_params.custom_llm_provider");
                deployment
                    .litellm_params
                    .custom_llm_provider
                    .map(|value| resolve_string(value, &path, lookup))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Settings { deployments })
    }
}

mod tests;
