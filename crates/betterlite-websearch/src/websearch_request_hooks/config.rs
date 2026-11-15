use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, validated_max_agentic_loops,
};
use serde::Deserialize;
use thiserror::Error;

const CEILING_FIELD: &str = "websearch_interception_params.max_agentic_loops";
const DEFAULT_PROVIDER: &str = "bedrock";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Ceiling(#[from] AgenticLoopCeilingError),
}

// LiteLLM's WebSearchInterceptionConfig TypedDict: absent and null both mean "use the default"
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebSearchInterceptionParams {
    #[serde(default)]
    pub enabled_providers: Option<Vec<String>>,
    #[serde(default)]
    pub search_tool_name: Option<String>,
    #[serde(default)]
    pub max_agentic_loops: Option<LoopValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LoopValue {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Text(String),
}

impl From<LoopValue> for AgenticLoopValue {
    fn from(value: LoopValue) -> Self {
        match value {
            LoopValue::Integer(integer) => Self::Integer(integer),
            LoopValue::Float(float) => Self::Float(float),
            LoopValue::Boolean(flag) => Self::Boolean(flag),
            LoopValue::Text(text) => Self::Text(text),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedParams {
    pub enabled_providers: Option<Vec<String>>,
    pub search_tool_name: Option<String>,
    pub max_agentic_loops: Option<u32>,
}

impl WebSearchInterceptionParams {
    pub fn resolve(self) -> Result<ResolvedParams, ConfigError> {
        // LiteLLM's __init__ defaults enabled_providers to bedrock and validates the ceiling through B4110
        let enabled_providers = Some(
            self.enabled_providers
                .unwrap_or_else(|| vec![DEFAULT_PROVIDER.to_owned()]),
        );
        let max_agentic_loops = match self.max_agentic_loops {
            Some(value) => {
                validated_max_agentic_loops(&AgenticLoopValue::from(value), CEILING_FIELD)?
            }
            None => None,
        };
        Ok(ResolvedParams {
            enabled_providers,
            search_tool_name: self.search_tool_name,
            max_agentic_loops,
        })
    }
}
