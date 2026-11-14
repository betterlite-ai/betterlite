use serde::Deserialize;

use crate::agentic_loop_ceiling::{
    AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS, validated_max_agentic_loops,
};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.max_agentic_loops";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawDeployment {
    #[serde(default)]
    pub litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawParams {
    #[serde(default)]
    pub max_agentic_loops: Option<RawNumber>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; every deployment gets a ceiling, the default when unset
    pub deployments: Vec<u32>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let path = format!("model_list.{index}.{PATH_ROOT}");
            deployments.push(resolve_ceiling(
                deployment.litellm_params.max_agentic_loops,
                &path,
                lookup,
            )?);
        }
        Ok(Settings { deployments })
    }
}

fn resolve_ceiling<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = match value {
        None => return Ok(DEFAULT_MAX_AGENTIC_LOOPS),
        Some(RawNumber::Text(text)) => RawNumber::Text(resolve_string(text, path, lookup)?),
        Some(other) => other,
    };
    let parsed = match value {
        RawNumber::Integer(integer) => AgenticLoopValue::Integer(integer),
        RawNumber::Float(float) => AgenticLoopValue::Float(float),
        RawNumber::Boolean(flag) => AgenticLoopValue::Boolean(flag),
        RawNumber::Text(text) => AgenticLoopValue::Text(text),
    };
    let ceiling = validated_max_agentic_loops(&parsed, path).map_err(|_source| {
        ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }
    })?;
    Ok(ceiling.unwrap_or(DEFAULT_MAX_AGENTIC_LOOPS))
}
