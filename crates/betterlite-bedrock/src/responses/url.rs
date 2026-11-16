use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{BedrockEndpointType, default_endpoint_url};
use crate::mantle_responses::url::BASE_SUFFIXES_TO_STRIP;
use crate::responses::error::ResponsesError;
use crate::signing::target::RegionInputs;

const RESPONSES_PATH: &str = "/openai/v1/responses";

#[derive(Debug, Clone, Copy, Default)]
pub struct CompleteUrlParams<'text> {
    pub api_base: Option<&'text str>,
    pub aws_region_name: Option<&'text str>,
    pub aws_bedrock_runtime_endpoint: Option<&'text str>,
    pub environment_endpoint: Option<&'text str>,
    pub region_inputs: RegionInputs<'text>,
}

pub fn complete_url(params: CompleteUrlParams<'_>) -> Result<String, ResponsesError> {
    let region = llm_region_name(LlmRegion {
        explicit: params.aws_region_name,
        model: None,
        model_id: None,
        environment_region_name: params.region_inputs.environment_region_name,
        environment_region: params.region_inputs.environment_region,
        session_region: params.region_inputs.session_region,
    })?;
    let host = [
        params.api_base,
        params.aws_bedrock_runtime_endpoint,
        params.environment_endpoint,
    ]
    .into_iter()
    .flatten()
    .find(|endpoint| !endpoint.is_empty())
    .map_or_else(
        || default_endpoint_url(BedrockEndpointType::Runtime, &region),
        str::to_owned,
    );
    let trimmed = host.trim_end_matches('/');
    let base = BASE_SUFFIXES_TO_STRIP
        .iter()
        .find_map(|suffix| trimmed.strip_suffix(suffix))
        .unwrap_or(trimmed);
    Ok(format!("{base}{RESPONSES_PATH}"))
}
