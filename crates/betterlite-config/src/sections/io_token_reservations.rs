use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    itpm: Option<RawNumber>,
    otpm: Option<RawNumber>,
    #[serde(default)]
    litellm_params: RawParams,
    #[serde(default)]
    model_info: RawInfo,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    itpm: Option<RawNumber>,
    otpm: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    itpm: Option<RawNumber>,
    otpm: Option<RawNumber>,
}

#[derive(Debug, Default)]
struct Levels {
    deployment: Option<RawNumber>,
    params: Option<RawNumber>,
    info: Option<RawNumber>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeploymentIoLimits {
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
}

// deployments stay index-aligned with model_list, like the router's config order
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Settings {
    pub deployments: Box<[DeploymentIoLimits]>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(deployment_limits(index, deployment, lookup)?);
        }
        Ok(Settings {
            deployments: deployments.into_boxed_slice(),
        })
    }
}

// LiteLLM's get_deployment_io_token_limits stops at the first level that holds a value
fn deployment_limits<L>(
    index: usize,
    deployment: RawDeployment,
    lookup: &L,
) -> Result<DeploymentIoLimits, ConfigError>
where
    L: EnvironmentLookup,
{
    let itpm = limit(
        index,
        "itpm",
        Levels {
            deployment: deployment.itpm,
            params: deployment.litellm_params.itpm,
            info: deployment.model_info.itpm,
        },
        lookup,
    )?;
    let otpm = limit(
        index,
        "otpm",
        Levels {
            deployment: deployment.otpm,
            params: deployment.litellm_params.otpm,
            info: deployment.model_info.otpm,
        },
        lookup,
    )?;
    Ok(DeploymentIoLimits { itpm, otpm })
}

fn limit<L>(
    index: usize,
    field: &str,
    levels: Levels,
    lookup: &L,
) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let deployment = resolve_number(
        levels.deployment,
        &format!("model_list.{index}.{field}"),
        lookup,
        parse_i64,
    )?;
    if deployment.is_some() {
        return Ok(deployment);
    }
    let params = resolve_number(
        levels.params,
        &format!("model_list.{index}.litellm_params.{field}"),
        lookup,
        parse_i64,
    )?;
    if params.is_some() {
        return Ok(params);
    }
    resolve_number(
        levels.info,
        &format!("model_list.{index}.model_info.{field}"),
        lookup,
        parse_i64,
    )
}

fn parse_i64(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
