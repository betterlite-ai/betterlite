use serde::Deserialize;

use crate::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, validated_max_agentic_loops,
};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const PARAMS_PATH: &str = "litellm_settings.websearch_interception_params";
const CALLBACK_PATH: &str = "callback_settings.websearch_interception";
const DEFAULT_PROVIDER: &str = "bedrock";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
    #[serde(default)]
    callback_settings: RawCallbackSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    #[serde(default)]
    websearch_interception_params: Option<RawParams>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCallbackSettings {
    #[serde(default)]
    websearch_interception: Option<RawParams>,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(default)]
    enabled_providers: Option<Vec<String>>,
    #[serde(default)]
    search_tool_name: Option<String>,
    #[serde(default)]
    max_agentic_loops: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawNumber {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled_providers: Vec<String>,
    pub search_tool_name: Option<String>,
    pub max_agentic_loops: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled_providers: vec![DEFAULT_PROVIDER.to_owned()],
            search_tool_name: None,
            max_agentic_loops: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM prefers litellm_settings.websearch_interception_params and falls back to the callback's own entry
        match self.litellm_settings.websearch_interception_params {
            Some(params) => convert_params(params, PARAMS_PATH, lookup),
            None => match self.callback_settings.websearch_interception {
                Some(params) => convert_params(params, CALLBACK_PATH, lookup),
                None => Ok(Settings::default()),
            },
        }
    }
}

fn convert_params<L>(params: RawParams, path: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let enabled_providers = params
        .enabled_providers
        .unwrap_or_else(|| vec![DEFAULT_PROVIDER.to_owned()]);
    let search_tool_name = params
        .search_tool_name
        .map(|name| resolve_string(name, &format!("{path}.search_tool_name"), lookup))
        .transpose()?;
    let max_agentic_loops = ceiling(params.max_agentic_loops, path, lookup)?;
    Ok(Settings {
        enabled_providers,
        search_tool_name,
        max_agentic_loops,
    })
}

fn ceiling<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<Option<u32>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = value else {
        return Ok(None);
    };
    let loop_value = match raw {
        RawNumber::Integer(integer) => AgenticLoopValue::Integer(integer),
        RawNumber::Float(float) => AgenticLoopValue::Float(float),
        RawNumber::Boolean(flag) => AgenticLoopValue::Boolean(flag),
        // an os.environ/ reference resolves to the variable's text, and an empty variable counts as unset
        RawNumber::Text(text) => {
            let resolved = resolve_string(text, &format!("{path}.max_agentic_loops"), lookup)?;
            if resolved.is_empty() {
                return Ok(None);
            }
            AgenticLoopValue::Text(resolved)
        }
    };
    let boolean = match &loop_value {
        AgenticLoopValue::Boolean(flag) => *flag,
        AgenticLoopValue::Null
        | AgenticLoopValue::Integer(_)
        | AgenticLoopValue::Float(_)
        | AgenticLoopValue::Text(_) => false,
    };
    let field = format!("{path}.max_agentic_loops");
    match validated_max_agentic_loops(&loop_value, &field) {
        Ok(ceiling) => Ok(ceiling),
        Err(AgenticLoopCeilingError::BelowMinimum { value, .. }) => {
            Err(ConfigError::InvalidScalar {
                path: format!("{path}.max_agentic_loops={value}, below the minimum of 1"),
                value: false,
            })
        }
        Err(AgenticLoopCeilingError::NotWholeNumber { .. }) => Err(ConfigError::InvalidScalar {
            path: field,
            value: boolean,
        }),
    }
}
