pub mod a2a;
mod tests;
use super::AgentResponse;
use super::json::AgentJson;
pub use a2a::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentCreateResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub _hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentDeleteResult {
    #[serde(default)]
    pub name: String,
    #[serde(default = "deleted_by_default")]
    pub deleted: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub _hidden_params: BTreeMap<String, AgentJson>,
}

fn deleted_by_default() -> bool {
    let default_deleted = true;
    default_deleted
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentListResponse {
    #[serde(default)]
    pub agents: Vec<BTreeMap<String, AgentJson>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub _hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentVersionsResponse {
    #[serde(default, rename = "agentVersions")]
    pub agent_versions: Vec<BTreeMap<String, AgentJson>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub _hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ListAgentsResponse {
    pub agents: Vec<AgentResponse>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentRequest {
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentHttpResponse {
    pub status_code: u16,
    pub body: AgentJson,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl AgentHttpResponse {
    pub fn decode<T>(&self) -> Result<T, AgentProviderError>
    where
        T: for<'de> Deserialize<'de>,
    {
        if !(200..300).contains(&self.status_code) {
            return Err(AgentProviderError::Http {
                status_code: self.status_code,
                message: self.body.to_string(),
                headers: self.headers.clone(),
            });
        }
        serde_json::from_value(self.body.clone())
            .map_err(|source| AgentProviderError::Decode { source })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentProviderError {
    #[error("agent provider returned HTTP {status_code}: {message}")]
    Http {
        status_code: u16,
        message: String,
        headers: BTreeMap<String, String>,
    },
    #[error("invalid agent provider response: {source}")]
    Decode { source: serde_json::Error },
    #[error("invalid agent provider response: {message}")]
    InvalidResponse { message: String },
}

impl AgentProviderError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Http { status_code, .. } => *status_code,
            Self::Decode { .. } | Self::InvalidResponse { .. } => 500,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Http { message, .. } | Self::InvalidResponse { message } => message.clone(),
            Self::Decode { source } => source.to_string(),
        }
    }
}

pub trait BaseAgentsAPIConfig {
    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<String, AgentProviderError>;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<BTreeMap<String, String>, AgentProviderError>;

    fn transform_create_request(
        &self,
        name: &str,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError>;

    fn transform_create_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, AgentProviderError>;

    fn transform_list_request(
        &self,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError>;

    fn transform_list_response(
        &self,
        response: &AgentHttpResponse,
    ) -> Result<AgentListResponse, AgentProviderError>;

    fn transform_get_request(
        &self,
        name: &str,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError>;

    fn transform_get_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, AgentProviderError>;

    fn transform_delete_request(
        &self,
        name: &str,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError>;

    fn transform_delete_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentDeleteResult, AgentProviderError>;

    fn transform_versions_request(
        &self,
        name: &str,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, AgentProviderError>;

    fn transform_versions_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentVersionsResponse, AgentProviderError>;

    fn get_error_class(
        error_message: &str,
        status_code: u16,
        headers: &[(String, String)],
    ) -> AgentProviderError
    where
        Self: Sized,
    {
        AgentProviderError::Http {
            status_code,
            message: String::from(error_message),
            headers: headers.iter().cloned().collect(),
        }
    }
}
