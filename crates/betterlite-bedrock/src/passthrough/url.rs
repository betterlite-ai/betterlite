use super::BedrockPassthroughPlan;
use super::error::PassthroughFailure;
use super::model_id::{encoded_model_id, with_model_id};
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use url::Url;
pub(super) fn optional_text(
    params: &Object,
    field: &'static str,
) -> Result<Option<String>, PassthroughFailure> {
    match params.get(field) {
        None => Ok(None),
        Some(value) => match value.as_str() {
            Some(text) => Ok(Some(text.to_owned())),
            None if value.is_null() => Ok(None),
            None => Err(PassthroughFailure::Field { field }),
        },
    }
}
pub fn complete_url(
    plan: &BedrockPassthroughPlan<'_>,
    request: &PassthroughUrlRequest<'_>,
) -> Result<(Url, String), PassthroughFailure> {
    let model_id = optional_text(request.litellm_params, "model_id")?;
    let explicit = optional_text(request.litellm_params, "aws_region_name")?;
    let configured_endpoint =
        optional_text(request.litellm_params, "aws_bedrock_runtime_endpoint")?;
    let environment = &plan.environment;
    let region = llm_region_name(LlmRegion {
        explicit: explicit.as_deref(),
        model: Some(request.target.model),
        model_id: model_id.as_deref(),
        environment_region_name: environment.region_name.as_deref(),
        environment_region: environment.region.as_deref(),
        session_region: environment.session_region.as_deref(),
    })?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: request.api_base,
        configured_endpoint: configured_endpoint.as_deref(),
        environment_endpoint: environment.runtime_endpoint.as_deref(),
        region: &region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    let rewritten = match &model_id {
        None => request.target.endpoint.to_owned(),
        Some(model_id) => with_model_id(request.target.endpoint, &encoded_model_id(model_id)),
    };
    let url = plan.format_url(
        &rewritten,
        &endpoint.endpoint_url,
        request.request_query_params,
    )?;
    Ok((url, endpoint.endpoint_url))
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassthroughEnvironment {
    pub region_name: Option<String>,
    pub region: Option<String>,
    pub session_region: Option<String>,
    pub runtime_endpoint: Option<String>,
}
