use serde::Deserialize;

use crate::agentic_loop_ceiling::{
    AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS, validated_max_agentic_loops,
};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const FIELD: &str = "litellm_params.max_agentic_loops";
const ENVIRONMENT_PREFIX: &str = "os.environ/";

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
    #[serde(default)]
    max_agentic_loops: Option<RawNumber>,
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
            let path = format!("model_list.{index}.{FIELD}");
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
    let Some(value) = value else {
        return Ok(DEFAULT_MAX_AGENTIC_LOOPS);
    };
    let parsed = match value {
        RawNumber::Text(text) => {
            let from_environment = text.starts_with(ENVIRONMENT_PREFIX);
            let resolved = resolve_string(text, path, lookup)?;
            // the shared parser counts an empty env var as unset, unlike an explicit empty scalar
            if from_environment && resolved.trim().is_empty() {
                return Ok(DEFAULT_MAX_AGENTIC_LOOPS);
            }
            AgenticLoopValue::Text(resolved)
        }
        RawNumber::Integer(integer) => AgenticLoopValue::Integer(integer),
        RawNumber::Float(float) => AgenticLoopValue::Float(float),
        RawNumber::Boolean(flag) => AgenticLoopValue::Boolean(flag),
    };
    let rejected_boolean = matches!(&parsed, AgenticLoopValue::Boolean(true));
    let ceiling = validated_max_agentic_loops(&parsed, FIELD).map_err(|_source| {
        ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: rejected_boolean,
        }
    })?;
    Ok(ceiling.unwrap_or(DEFAULT_MAX_AGENTIC_LOOPS))
}
