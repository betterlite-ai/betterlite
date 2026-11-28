use betterlite_bedrock::image_call::execute::{dispatch_request, transform_response};
use betterlite_bedrock::image_call::plan::{
    CredentialSource, ImageCall, LogContext, prepare_request, resolve_target,
};
use betterlite_bedrock::image_call::transport::ClientFactory;
use betterlite_bedrock::image_models::response::ModelResponse;

use super::super::call::{GenerateOperation, Transport};
use super::super::error::BedrockImageError;

// LiteLLM's generation arm hands the Bedrock handler its call; the native leaf prepares, sends and projects it
pub(super) async fn generate<Source, Factory>(
    operation: &GenerateOperation<'_>,
    source: &Source,
    transport: Transport<'_, Factory>,
    log: &mut LogContext<'_, '_>,
) -> Result<ModelResponse, BedrockImageError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
{
    let Some(model) = operation.model else {
        return Err(BedrockImageError::ModelRequired);
    };
    // LiteLLM's generation arm forwards neither headers nor extra_headers to the Bedrock handler
    let mut call = ImageCall {
        model,
        prompt: operation.prompt,
        optional_params: operation.optional_params.clone(),
        api_base: operation.api_base,
        extra_headers: None,
        api_key: operation.api_key,
        timeout: operation.timeout,
        aimg_generation: operation.aimg_generation,
        environment: operation.environment,
        now: operation.now,
    };
    let target = resolve_target(&mut call, source).await?;
    let prepared = prepare_request(&mut call, &target, log)?;
    let reply = dispatch_request(&call, &prepared, transport.supplied, transport.factory).await?;
    let response = ModelResponse {
        created: Some(operation.created),
        data: Vec::new(),
    };
    transform_response(&call, &prepared, &reply, response, log).map_err(BedrockImageError::from)
}
