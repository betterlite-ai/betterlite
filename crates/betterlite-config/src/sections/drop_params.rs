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

// the policy lives behind a box so Sections stays small; betterlite-db-admin holds a Config across an await
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub policy: Box<DropPolicySettings>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropPolicySettings {
    pub global_drop_params: bool,
    pub deployments: Box<[DeploymentDropPolicy]>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment_text = lookup.lookup("LITELLM_DROP_PARAMS").unwrap_or_default();
        let environment = betterlite_env::flag("LITELLM_DROP_PARAMS", &environment_text)?;
        let mut paths = Vec::new();
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let additional_drop_params = deployment
                .litellm_params
                .additional_drop_params
                .unwrap_or_default();
            // LiteLLM reads a dotted or bracketed entry as a JSON path; the path engine is B213
            paths.extend(
                additional_drop_params
                    .iter()
                    .filter(|entry| entry.contains('.') || entry.contains('['))
                    .map(|entry| {
                        format!("model_list.{index}.litellm_params.additional_drop_params[{entry}]")
                    }),
            );
            deployments.push(DeploymentDropPolicy {
                drop_params: deployment.litellm_params.drop_params,
                additional_drop_params,
            });
        }
        if !paths.is_empty() {
            return Err(ConfigError::Unsupported { paths });
        }
        Ok(Settings {
            policy: Box::new(DropPolicySettings {
                global_drop_params: self
                    .litellm_settings
                    .drop_params
                    .or(environment)
                    .unwrap_or(false),
                deployments: deployments.into_boxed_slice(),
            }),
        })
    }
}
