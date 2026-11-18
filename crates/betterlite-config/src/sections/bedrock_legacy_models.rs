pub mod families;
mod tests;
pub mod values;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use values::{RawValues, model_values};

pub type Settings = values::Settings;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawValues,
}

impl Section for Raw {
    type Settings = values::Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params");
            deployments.push(model_values(deployment.litellm_params, &prefix, lookup)?);
        }
        Ok(values::Settings {
            deployments: deployments.into_boxed_slice(),
            litellm_settings: Box::new(model_values(
                self.litellm_settings,
                "litellm_settings",
                lookup,
            )?),
        })
    }
}
