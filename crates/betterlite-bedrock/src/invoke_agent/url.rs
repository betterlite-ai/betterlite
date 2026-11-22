use betterlite_http::url_path::encode_segment;
use uuid::Uuid;

use super::error::InvokeAgentRequestError;
use super::identifiers::{agent_id_and_alias_id, session_id};
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::model_routes::{ParamMap, ParamValue};
use crate::signing::target::RegionInputs;

#[derive(Debug, Clone, Copy, Default)]
pub struct InvokeEnvironment<'text> {
    pub runtime_endpoint: Option<&'text str>,
    pub region_inputs: RegionInputs<'text>,
}

#[derive(Debug, Clone, Copy)]
pub struct CompleteUrl<'text> {
    pub api_base: Option<&'text str>,
    pub model: &'text str,
    pub optional_params: &'text ParamMap,
    pub environment: InvokeEnvironment<'text>,
    pub generated_session_id: Uuid,
}

pub fn complete_url(request: CompleteUrl<'_>) -> Result<String, InvokeAgentRequestError> {
    let region = llm_region_name(LlmRegion {
        explicit: optional_text(request.optional_params, "aws_region_name")?,
        model: Some(request.model),
        model_id: None,
        environment_region_name: request.environment.region_inputs.environment_region_name,
        environment_region: request.environment.region_inputs.environment_region,
        session_region: request.environment.region_inputs.session_region,
    })?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: request.api_base,
        configured_endpoint: optional_text(
            request.optional_params,
            "aws_bedrock_runtime_endpoint",
        )?,
        environment_endpoint: request.environment.runtime_endpoint,
        region: &region,
        endpoint_type: BedrockEndpointType::Agent,
    });
    let (agent_id, alias_id) = agent_id_and_alias_id(request.model)?;
    let session = session_id(request.optional_params, request.generated_session_id)?;
    let encoded_agent_id = encode_segment(agent_id, "agent_id")?;
    let encoded_agent_alias_id = encode_segment(alias_id, "agent_alias_id")?;
    let encoded_session_id = encode_segment(&session, "session_id")?;
    Ok(format!(
        "{}/agents/{encoded_agent_id}/agentAliases/{encoded_agent_alias_id}/sessions/{encoded_session_id}/text",
        endpoint.endpoint_url
    ))
}

fn optional_text<'params>(
    optional_params: &'params ParamMap,
    field: &'static str,
) -> Result<Option<&'params str>, InvokeAgentRequestError> {
    match optional_params.get(field) {
        None | Some(ParamValue::Null) => Ok(None),
        Some(ParamValue::Text(text)) => Ok(Some(text)),
        Some(_) => Err(InvokeAgentRequestError::WrongType { field }),
    }
}
