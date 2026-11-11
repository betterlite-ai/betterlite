use std::collections::BTreeMap;

use thiserror::Error;

use super::config::Params;
use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_endpoints::model_id::bedrock_model_id;
use crate::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};

#[derive(Debug, Clone, Copy, Default)]
pub struct InvokeEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
    pub session_region: Option<&'text str>,
    pub runtime_endpoint: Option<&'text str>,
}

#[derive(Debug, Clone, Copy)]
pub struct CompleteUrl<'text> {
    pub api_base: Option<&'text str>,
    pub model: &'text str,
    pub optional_params: &'text Params,
    pub stream: bool,
    pub environment: InvokeEnvironment<'text>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeModelUrl {
    pub endpoint_url: String,
    pub model_id: String,
    pub provider: Option<BedrockInvokeProvider>,
    pub region: String,
}

#[derive(Debug, Error)]
pub enum UrlError {
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error("field {field} is not a string")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

pub fn invoke_model_url(request: CompleteUrl<'_>) -> Result<InvokeModelUrl, UrlError> {
    let provider = invoke_provider(request.model);
    // the model id override is the only parameter the shared encoder reads
    let mut model_id_params = BTreeMap::new();
    if let Some(model_id) = request.optional_params.get("model_id") {
        model_id_params.insert("model_id".to_owned(), model_id.clone());
    }
    let model_id = bedrock_model_id(&mut model_id_params, provider, request.model)?;
    let configured_endpoint = read_string(request.optional_params, "aws_bedrock_runtime_endpoint")?;
    let explicit_region = read_string(request.optional_params, "aws_region_name")?;
    // LiteLLM pops the model id before this call, so an override cannot supply the region
    let region = llm_region_name(LlmRegion {
        explicit: explicit_region.as_deref(),
        model: Some(request.model),
        model_id: None,
        environment_region_name: request.environment.region_name,
        environment_region: request.environment.region,
        session_region: request.environment.session_region,
    })?;
    let base = runtime_endpoint(RuntimeEndpointInput {
        api_base: request.api_base,
        configured_endpoint: configured_endpoint.as_deref(),
        environment_endpoint: request.environment.runtime_endpoint,
        region: &region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    let streaming = request.stream && provider != Some(BedrockInvokeProvider::Ai21);
    let suffix = if streaming {
        "invoke-with-response-stream"
    } else {
        "invoke"
    };
    Ok(InvokeModelUrl {
        endpoint_url: format!("{}/model/{model_id}/{suffix}", base.endpoint_url),
        model_id,
        provider,
        region,
    })
}

fn read_string(params: &Params, field: &'static str) -> Result<Option<String>, UrlError> {
    params.get(field).map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<String>>(raw.get())
            .map_err(|source| UrlError::Field { field, source })
    })
}
