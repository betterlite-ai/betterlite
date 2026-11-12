use betterlite_http::url_path::encode_segment;

use crate::aws_endpoints::region::non_llm_region_name;
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::vector_stores::error::VectorStoreError;

const KNOWLEDGE_BASES_SUFFIX: &str = "/knowledgebases";

pub struct CompleteUrl<'request> {
    pub api_base: Option<&'request str>,
    pub configured_endpoint: Option<&'request str>,
    pub environment_endpoint: Option<&'request str>,
    pub aws_region_name: Option<&'request str>,
    pub environment_region_name: Option<&'request str>,
    pub environment_region: Option<&'request str>,
}

pub fn complete_url(request: &CompleteUrl<'_>) -> Result<String, VectorStoreError> {
    let region = non_llm_region_name(
        request.aws_region_name,
        request.environment_region_name,
        request.environment_region,
    )?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: request.api_base,
        configured_endpoint: request.configured_endpoint,
        environment_endpoint: request.environment_endpoint,
        region: &region,
        endpoint_type: BedrockEndpointType::Agent,
    });
    Ok(format!("{}{KNOWLEDGE_BASES_SUFFIX}", endpoint.endpoint_url))
}

pub fn retrieve_url(api_base: &str, vector_store_id: &str) -> Result<String, VectorStoreError> {
    let encoded = encode_segment(vector_store_id, "vector_store_id")?;
    Ok(format!("{api_base}/{encoded}/retrieve"))
}
