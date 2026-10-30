pub mod a2a;
mod request_id;
mod tests;
use super::AgentResponse;
use super::json::AgentJson;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentCreateResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentDeleteResult {
    pub name: String,
    #[serde(default)]
    pub deleted: Deleted,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Deleted(pub bool);

impl Default for Deleted {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentListResponse {
    #[serde(default)]
    pub agents: Vec<BTreeMap<String, AgentJson>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub hidden_params: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentVersionsResponse {
    #[serde(default)]
    pub agent_versions: Vec<BTreeMap<String, AgentJson>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
    #[serde(skip)]
    pub hidden_params: BTreeMap<String, AgentJson>,
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
    #[serde(default)]
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
    #[error("cannot format A2A request id: {source}")]
    Format { source: fmt::Error },
    #[error("invalid agent provider response: {message}")]
    InvalidResponse { message: String },
}

impl AgentProviderError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Http { status_code, .. } => *status_code,
            Self::Decode { .. } | Self::Format { .. } | Self::InvalidResponse { .. } => 500,
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Http { message, .. } | Self::InvalidResponse { message } => message.clone(),
            Self::Decode { source } => source.to_string(),
            Self::Format { source } => source.to_string(),
        }
    }
}

pub trait BaseAgentsAPIConfig {
    type Error: From<AgentProviderError>;

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<String, Self::Error>;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<BTreeMap<String, String>, Self::Error>;

    fn transform_create_request(
        &self,
        name: &str,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, Self::Error>;

    fn transform_create_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, Self::Error>;

    fn transform_list_request(
        &self,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, Self::Error>;

    fn transform_list_response(
        &self,
        response: &AgentHttpResponse,
    ) -> Result<AgentListResponse, Self::Error>;

    fn transform_get_request(
        &self,
        name: &str,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, Self::Error>;

    fn transform_get_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, Self::Error>;

    fn transform_delete_request(
        &self,
        name: &str,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, Self::Error>;

    fn transform_delete_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentDeleteResult, Self::Error>;

    fn transform_versions_request(
        &self,
        name: &str,
        api_base: Option<&str>,
        parameters: &BTreeMap<String, AgentJson>,
    ) -> Result<AgentRequest, Self::Error>;

    fn transform_versions_response(
        &self,
        response: &AgentHttpResponse,
        name: &str,
    ) -> Result<AgentVersionsResponse, Self::Error>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &BTreeMap<String, String>,
    ) -> Self::Error {
        Self::Error::from(AgentProviderError::Http {
            status_code,
            message: String::from(error_message),
            headers: headers.clone(),
        })
    }
}
