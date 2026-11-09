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

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params");
            let tags = deployment
                .litellm_params
                .tags
                .map(|RawTags::List(values)| values);
            let tag_regex = deployment
                .litellm_params
                .tag_regex
                .map(|RawTagPatterns::List(values)| values);
            deployments.push(DeploymentTags {
                tags: resolve_list(tags, &format!("{prefix}.tags"), lookup)?,
                tag_regex: resolve_list(tag_regex, &format!("{prefix}.tag_regex"), lookup)?,
            });
        }
        Ok(Settings { deployments })
    }
}

fn resolve_list<L>(
    configured: Option<Vec<String>>,
    path: &str,
    lookup: &L,
) -> Result<Option<Vec<String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(entries) = configured else {
        return Ok(None);
    };
    let mut resolved = Vec::with_capacity(entries.len());
    for (index, value) in entries.into_iter().enumerate() {
        resolved.push(resolve_string(value, &format!("{path}.{index}"), lookup)?);
    }
    Ok(Some(resolved))
}

mod tests;
