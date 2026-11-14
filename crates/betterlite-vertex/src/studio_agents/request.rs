use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[derive(Debug, Clone)]
pub struct AgentParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub base_agent: Option<String>,
    pub instructions: Option<String>,
    pub base_environment: Option<BaseEnvironment>,
    pub page_size: Option<i64>,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BaseEnvironment {
    Id(String),
    Object(IndexMap<String, Json>),
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub base_agent: Option<String>,
    pub instructions: Option<String>,
    pub base_environment: Option<BaseEnvironment>,
}

#[must_use]
pub fn create_agent_body(name: &str, params: &AgentParams) -> CreateAgentRequest {
    CreateAgentRequest {
        name: name.to_owned(),
        base_agent: params.base_agent.clone(),
        instructions: params.instructions.clone(),
        base_environment: params.base_environment.clone(),
    }
}
