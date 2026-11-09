use betterlite_context::bedrock_model::get_bedrock_base_model;
use percent_encoding::utf8_percent_encode;

use crate::aws_endpoints::model_id::MODEL_ID_SAFE;
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};

#[derive(Debug, Clone, Copy)]
pub struct CountTokensEndpoint<'text> {
    pub model: &'text str,
    pub aws_region_name: &'text str,
    pub api_base: Option<&'text str>,
    pub aws_bedrock_runtime_endpoint: Option<&'text str>,
    pub environment_endpoint: Option<&'text str>,
}

#[must_use]
pub fn count_tokens_endpoint(input: CountTokensEndpoint<'_>) -> String {
    let base_model = get_bedrock_base_model(input.model);
    let model_id = base_model.strip_prefix("bedrock/").unwrap_or(&base_model);
    let encoded = utf8_percent_encode(model_id, &MODEL_ID_SAFE);
    let base_endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: input.api_base,
        configured_endpoint: input.aws_bedrock_runtime_endpoint,
        environment_endpoint: input.environment_endpoint,
        region: input.aws_region_name,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    format!(
        "{}/model/{encoded}/count-tokens",
        base_endpoint.endpoint_url
    )
}
