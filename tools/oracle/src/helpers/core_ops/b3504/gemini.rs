use betterlite_config::sections::studio_identity::Settings;
use betterlite_core::api::agents::collection::AgentFailure;
use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, UpstreamResponse,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_vertex::studio_agents::decode::{
    AgentOperation, AgentPayload, RawResponse, decode_studio_agent_payload,
};
use betterlite_vertex::studio_agents::error::StudioAgentError;
use betterlite_vertex::studio_agents::headers::AgentHeaders;
use betterlite_vertex::studio_agents::headers::validate_environment;
use betterlite_vertex::studio_agents::request::{AgentParams, BaseEnvironment, create_agent_body};
use betterlite_vertex::studio_agents::urls::{collection_target, item_target};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use thiserror::Error;

// LiteLLM's exception_type reads status_code and message off the GeminiError the config raises
#[derive(Debug, Error)]
pub(crate) enum AgentError {
    #[error(transparent)]
    Studio(#[from] StudioAgentError),
    #[error("{message}")]
    Refused { message: String, status: u16 },
    #[error("the agent params do not have the shape the config reads")]
    Params,
    #[error("the provider reply does not have the shape the operation reads")]
    Shape,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl AgentFailure for AgentError {
    fn status_code(&self) -> Option<u16> {
        match self {
            Self::Studio(StudioAgentError::Gemini { status, .. }) => Some(*status),
            Self::Refused { status, .. } => Some(*status),
            Self::Studio(_) | Self::Params | Self::Shape | Self::Json(_) => None,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Studio(StudioAgentError::Gemini { body, .. }) => body.clone(),
            Self::Refused { message, .. } => message.clone(),
            Self::Studio(source) => source.to_string(),
            Self::Params | Self::Shape | Self::Json(_) => self.to_string(),
        }
    }
}

// Gemini's native agents API as B1392 implements it, wired to the call pipeline the way LiteLLM's config is
pub(crate) struct GeminiConfig {
    settings: Settings,
}

impl GeminiConfig {
    pub(crate) fn new() -> Self {
        Self {
            settings: Settings::default(),
        }
    }
}

impl ErrorClasses for GeminiConfig {
    type Error = AgentError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &betterlite_http::interactions::base::ResponseHeaders,
    ) -> AgentError {
        AgentError::Refused {
            message: message.to_owned(),
            status: status_code,
        }
    }
}

impl AgentsApiConfig for GeminiConfig {
    type CreateOutput = Value;
    type ListOutput = Value;
    type GetOutput = Value;

    fn validate_environment(
        &self,
        headers: &std::collections::BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, AgentError> {
        let validated: AgentHeaders =
            validate_environment(headers, &agent_params(litellm_params)?, &self.settings)?;
        Ok(validated.into_iter().collect())
    }

    fn get_complete_url(
        &self,
        _api_base: &str,
        litellm_params: &Params,
    ) -> Result<String, AgentError> {
        Ok(collection_target(&agent_params(litellm_params)?, &self.settings).url)
    }

    fn transform_create_request(
        &self,
        name: &str,
        litellm_params: &Params,
    ) -> Result<Params, AgentError> {
        let body = create_agent_body(name, &agent_params(litellm_params)?);
        Ok(serde_json::from_value(serde_json::to_value(body)?)?)
    }

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<Value, AgentError> {
        match decoded(response, AgentOperation::Create, name)? {
            AgentPayload::Agent(agent) => Ok(serde_json::to_value(agent)?),
            AgentPayload::Agents(_) | AgentPayload::Versions(_) => Err(AgentError::Shape),
        }
    }

    fn transform_list_request(
        &self,
        _api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, AgentError> {
        let target = collection_target(&agent_params(litellm_params)?, &self.settings);
        Ok(PreparedRequest {
            url: target.url,
            params: target
                .query
                .into_iter()
                .map(|(name, value)| (name, JsonValue::Text(value)))
                .collect(),
        })
    }

    fn transform_list_response(&self, response: &UpstreamResponse) -> Result<Value, AgentError> {
        match decoded(response, AgentOperation::List, "")? {
            AgentPayload::Agents(page) => Ok(serde_json::to_value(page)?),
            AgentPayload::Agent(_) | AgentPayload::Versions(_) => Err(AgentError::Shape),
        }
    }

    fn transform_get_request(
        &self,
        name: &str,
        _api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, AgentError> {
        let target = item_target(name, &agent_params(litellm_params)?, &self.settings);
        Ok(PreparedRequest {
            url: target.url,
            params: Params::new(),
        })
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<Value, AgentError> {
        match decoded(response, AgentOperation::Get, name)? {
            AgentPayload::Agent(agent) => Ok(serde_json::to_value(agent)?),
            AgentPayload::Agents(_) | AgentPayload::Versions(_) => Err(AgentError::Shape),
        }
    }
}

fn decoded(
    response: &UpstreamResponse,
    operation: AgentOperation,
    name: &str,
) -> Result<AgentPayload, AgentError> {
    let headers: IndexMap<String, String> = response
        .headers
        .iter()
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect();
    let raw = RawResponse {
        status: response.status_code,
        headers: &headers,
        body: &response.body,
    };
    Ok(decode_studio_agent_payload(&raw, operation, name)?.payload)
}

fn agent_params(litellm_params: &Params) -> Result<AgentParams, AgentError> {
    Ok(AgentParams {
        api_key: text(litellm_params, "api_key")?.map(SecretString::from),
        api_base: text(litellm_params, "api_base")?,
        base_agent: text(litellm_params, "base_agent")?,
        instructions: text(litellm_params, "instructions")?,
        base_environment: base_environment(litellm_params.get("base_environment"))?,
        page_size: page_size(litellm_params.get("page_size"))?,
        page_token: text(litellm_params, "page_token")?,
    })
}

fn text(params: &Params, name: &str) -> Result<Option<String>, AgentError> {
    match params.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(value)) => Ok(Some(value.clone())),
        Some(_other) => Err(AgentError::Params),
    }
}

fn base_environment(value: Option<&JsonValue>) -> Result<Option<BaseEnvironment>, AgentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(environment) => Ok(Some(serde_json::from_value(serde_json::to_value(
            environment,
        )?)?)),
    }
}

fn page_size(value: Option<&JsonValue>) -> Result<Option<i64>, AgentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => number.as_i64().map(Some).ok_or(AgentError::Params),
        Some(_other) => Err(AgentError::Params),
    }
}
