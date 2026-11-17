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
    disable_add_prefix_to_prompt: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    disable_add_prefix_to_prompt: Option<bool>,
}

// Boxed: Sections holds each Settings by value, and config snapshots live inside futures
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub inner: Box<Inner>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inner {
    pub global_disable_prefix: bool,
    pub deployment_disable_prefix: Vec<bool>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let name = "LITELLM_DISABLE_ADD_PREFIX_TO_PROMPT";
        let environment = betterlite_env::flag(name, &lookup.lookup(name).unwrap_or_default())?;
        let global_disable_prefix = self
            .litellm_settings
            .disable_add_prefix_to_prompt
            .or(environment)
            .unwrap_or(false);
        let deployment_disable_prefix = self
            .model_list
            .into_iter()
            .map(|deployment| {
                deployment
                    .litellm_params
                    .disable_add_prefix_to_prompt
                    .unwrap_or(global_disable_prefix)
            })
            .collect();
        Ok(Settings {
            inner: Box::new(Inner {
                global_disable_prefix,
                deployment_disable_prefix,
            }),
        })
    }
}
