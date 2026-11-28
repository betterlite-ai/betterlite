pub mod error;
mod params;
mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::agents_delete_versions::{
    AgentsConfig, EnvironmentRequest, NamedAgentRequest,
};
use betterlite_http::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, ResponseHeaders, UpstreamResponse,
};
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::decode::{
    AgentOperation, AgentPayload, DecodedPayload, RawResponse, decode_studio_agent_delete,
    decode_studio_agent_payload,
};
use super::error::StudioAgentError;
use super::headers;
use super::records::{AgentDeleteResult, AgentPage, AgentVersionsPage, StudioAgent};
use super::request::create_agent_body;
use super::urls::{AgentTarget, collection_target, item_target, versions_target};
use error::GeminiAgentError;
use params::agent_params;

#[derive(Debug, Clone)]
pub struct GeminiAgentsConfig {
    pub settings: Settings,
}

impl GeminiAgentsConfig {
    fn agent_headers(
        &self,
        caller: &BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, GeminiAgentError> {
        let params = agent_params(litellm_params)?;
        let headers = headers::validate_environment(caller, &params, &self.settings)?;
        Ok(headers.into_iter().collect())
    }
}

impl ErrorClasses for GeminiAgentsConfig {
    type Error = GeminiAgentError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> GeminiAgentError {
        GeminiAgentError::Status {
            status: status_code,
            message: message.to_owned(),
            headers: headers.clone(),
        }
    }
}

impl AgentsApiConfig for GeminiAgentsConfig {
    type CreateOutput = StudioAgent;
    type ListOutput = AgentPage;
    type GetOutput = StudioAgent;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, GeminiAgentError> {
        self.agent_headers(headers, litellm_params)
    }

    fn get_complete_url(
        &self,
        _api_base: &str,
        litellm_params: &Params,
    ) -> Result<String, GeminiAgentError> {
        let target = collection_target(&agent_params(litellm_params)?, &self.settings);
        Ok(target.url)
    }

    fn transform_create_request(
        &self,
        name: &str,
        litellm_params: &Params,
    ) -> Result<Params, GeminiAgentError> {
        let params = agent_params(litellm_params)?;
        let body = create_agent_body(name, &params);
        let Ok(text) = serde_json::to_string(&body) else {
            return Err(GeminiAgentError::Params {
                field: "base_environment",
            });
        };
        serde_json::from_str(&text).map_err(|_error| GeminiAgentError::Params {
            field: "base_environment",
        })
    }

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<StudioAgent, GeminiAgentError> {
        expected(response, AgentOperation::Create, name)
    }

    fn transform_list_request(
        &self,
        _api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, GeminiAgentError> {
        let params = agent_params(litellm_params)?;
        Ok(prepared_request(collection_target(&params, &self.settings)))
    }

    fn transform_list_response(
        &self,
        response: &UpstreamResponse,
    ) -> Result<AgentPage, GeminiAgentError> {
        expected(response, AgentOperation::List, "")
    }

    fn transform_get_request(
        &self,
        name: &str,
        _api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, GeminiAgentError> {
        let params = agent_params(litellm_params)?;
        Ok(prepared_request(item_target(name, &params, &self.settings)))
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<StudioAgent, GeminiAgentError> {
        expected(response, AgentOperation::Get, name)
    }
}

impl AgentsConfig for GeminiAgentsConfig {
    type DeleteOutput = AgentDeleteResult;
    type VersionsOutput = AgentVersionsPage;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, GeminiAgentError> {
        let caller: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.agent_headers(&caller, request.litellm_params)
    }

    fn transform_delete_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<String, GeminiAgentError> {
        let params = agent_params(request.litellm_params)?;
        Ok(item_target(request.name, &params, &self.settings).url)
    }

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentDeleteResult, GeminiAgentError> {
        let headers = reply_headers(response);
        let raw = RawResponse {
            status: response.status_code,
            headers: &headers,
            body: &response.body,
        };
        decode_studio_agent_delete(&raw, name).map_err(GeminiAgentError::Studio)
    }

    fn transform_versions_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<PreparedRequest, GeminiAgentError> {
        let target = versions_target(
            request.name,
            &agent_params(request.litellm_params)?,
            &self.settings,
        );
        Ok(prepared_request(target))
    }

    fn transform_versions_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentVersionsPage, GeminiAgentError> {
        expected(response, AgentOperation::Versions, name)
    }
}

fn prepared_request(target: AgentTarget) -> PreparedRequest {
    let params = query_params(target.query);
    PreparedRequest {
        url: target.url,
        params,
    }
}

fn query_params(pairs: Vec<(String, String)>) -> Params {
    pairs
        .into_iter()
        .map(|(name, value)| (name, JsonValue::Text(value)))
        .collect()
}

fn decoded(
    response: &UpstreamResponse,
    operation: AgentOperation,
    name: &str,
) -> Result<DecodedPayload, GeminiAgentError> {
    let headers = reply_headers(response);
    let raw = RawResponse {
        status: response.status_code,
        headers: &headers,
        body: &response.body,
    };
    decode_studio_agent_payload(&raw, operation, name).map_err(GeminiAgentError::Studio)
}

// the reply shape each operation reads out of the decoded body
trait PayloadOf: Sized {
    fn from_payload(payload: AgentPayload) -> Option<Self>;
}

impl PayloadOf for StudioAgent {
    fn from_payload(payload: AgentPayload) -> Option<Self> {
        match payload {
            AgentPayload::Agent(agent) => Some(agent),
            AgentPayload::Agents(_) | AgentPayload::Versions(_) => None,
        }
    }
}

impl PayloadOf for AgentPage {
    fn from_payload(payload: AgentPayload) -> Option<Self> {
        match payload {
            AgentPayload::Agents(page) => Some(page),
            AgentPayload::Agent(_) | AgentPayload::Versions(_) => None,
        }
    }
}

impl PayloadOf for AgentVersionsPage {
    fn from_payload(payload: AgentPayload) -> Option<Self> {
        match payload {
            AgentPayload::Versions(page) => Some(page),
            AgentPayload::Agent(_) | AgentPayload::Agents(_) => None,
        }
    }
}

fn expected<T>(
    response: &UpstreamResponse,
    operation: AgentOperation,
    name: &str,
) -> Result<T, GeminiAgentError>
where
    T: PayloadOf,
{
    let payload = decoded(response, operation, name)?.payload;
    T::from_payload(payload)
        .ok_or_else(|| GeminiAgentError::Studio(StudioAgentError::Shape { field: "root" }))
}

fn reply_headers(response: &UpstreamResponse) -> IndexMap<String, String> {
    response
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
