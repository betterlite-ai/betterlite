use std::collections::BTreeMap;

use betterlite_core::agents::provider::{
    AgentCreateResponse, AgentDeleteResult, AgentVersionsResponse, Deleted,
};
use betterlite_core::api::agents::item::error::ProviderFailure;
use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::agents_delete_versions::{
    AgentsConfig, EnvironmentRequest, NamedAgentRequest,
};
use betterlite_http::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, ResponseHeaders, UpstreamResponse,
};
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::records::{VersionsWire, agent_response, ensure_success, gemini_headers, page_params};

#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct FixtureError {
    pub(super) status: u16,
    pub(super) message: String,
}

impl ProviderFailure for FixtureError {
    fn status_code(&self) -> u16 {
        self.status
    }

    fn message(&self) -> String {
        self.message.clone()
    }
}

// the Gemini agents config's transforms; the provider itself is another task's, so the fixture stands in for it
pub(super) struct FixtureConfig;

impl ErrorClasses for FixtureConfig {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> FixtureError {
        FixtureError {
            status: status_code,
            message: message.to_owned(),
        }
    }
}

impl AgentsApiConfig for FixtureConfig {
    type CreateOutput = AgentCreateResponse;
    type ListOutput = AgentCreateResponse;
    type GetOutput = AgentCreateResponse;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, FixtureError> {
        Ok(gemini_headers(headers, litellm_params))
    }

    fn get_complete_url(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<String, FixtureError> {
        Ok(format!("{api_base}/v1beta/agents"))
    }

    fn transform_create_request(
        &self,
        _name: &str,
        _litellm_params: &Params,
    ) -> Result<Params, FixtureError> {
        Ok(Params::new())
    }

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, FixtureError> {
        agent_response(response, name)
    }

    fn transform_list_request(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(PreparedRequest {
            url: format!("{api_base}/v1beta/agents"),
            params: Params::new(),
        })
    }

    fn transform_list_response(
        &self,
        _response: &UpstreamResponse,
    ) -> Result<AgentCreateResponse, FixtureError> {
        Ok(AgentCreateResponse::default())
    }

    fn transform_get_request(
        &self,
        name: &str,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(PreparedRequest {
            url: format!("{api_base}/v1beta/agents/{name}"),
            params: Params::new(),
        })
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, FixtureError> {
        agent_response(response, name)
    }
}

impl AgentsConfig for FixtureConfig {
    type DeleteOutput = AgentDeleteResult;
    type VersionsOutput = AgentVersionsResponse;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        let extra: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        Ok(gemini_headers(&extra, request.litellm_params))
    }

    fn transform_delete_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<String, FixtureError> {
        Ok(format!(
            "{}/v1beta/agents/{}",
            request.api_base, request.name
        ))
    }

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentDeleteResult, FixtureError> {
        ensure_success(response)?;
        Ok(AgentDeleteResult {
            name: name.to_owned(),
            deleted: Deleted(true),
            extra: BTreeMap::new(),
            hidden_params: BTreeMap::new(),
        })
    }

    fn transform_versions_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(PreparedRequest {
            url: format!(
                "{}/v1beta/agents/{}/versions",
                request.api_base, request.name
            ),
            params: page_params(request.litellm_params),
        })
    }

    fn transform_versions_response(
        &self,
        response: &UpstreamResponse,
        _name: &str,
    ) -> Result<AgentVersionsResponse, FixtureError> {
        ensure_success(response)?;
        let wire: VersionsWire = serde_json::from_slice(&response.body).unwrap_or_default();
        Ok(AgentVersionsResponse {
            agent_versions: wire.agent_versions,
            next_page_token: wire.next_page_token,
            ..Default::default()
        })
    }
}
