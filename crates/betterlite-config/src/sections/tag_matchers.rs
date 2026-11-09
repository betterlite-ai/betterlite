use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
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
    tags: Option<RawTags>,
    tag_regex: Option<RawTagPatterns>,
}

#[derive(Debug, Deserialize)]
#[serde(
    untagged,
    expecting = "a list of tags in model_list[].litellm_params.tags"
)]
enum RawTags {
    List(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(
    untagged,
    expecting = "a list of patterns in model_list[].litellm_params.tag_regex"
)]
enum RawTagPatterns {
    List(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentTags {
    pub tags: Option<Vec<String>>,
    pub tag_regex: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Vec<DeploymentTags>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM resolves os.environ/ only in mapping values, so a tag or pattern string stays as written
        let deployments = self
            .model_list
            .into_iter()
            .map(|deployment| DeploymentTags {
                tags: deployment
                    .litellm_params
                    .tags
                    .map(|RawTags::List(values)| values),
                tag_regex: deployment
                    .litellm_params
                    .tag_regex
                    .map(|RawTagPatterns::List(values)| values),
            })
            .collect();
        Ok(Settings { deployments })
    }
}

mod tests;
