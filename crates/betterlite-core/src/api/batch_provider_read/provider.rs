use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::sdk::Timeout;

use super::call::{
    BatchProviderReadCall, BedrockAsyncInvokeReadCall, BedrockModelInvocationJobReadCall,
};
use super::error::ReadCallError;
use super::resolution;
use super::{BatchProviderReadInput, BatchReadProvider};

const DEFAULT_BEDROCK_REGION: &str = "us-east-1";

// batch ids that carry a Bedrock ARN are read from AWS rather than from a provider batch endpoint
pub(super) fn bedrock_arn_call(input: &BatchProviderReadInput) -> Option<BatchProviderReadCall> {
    let kind = BedrockArn::parse(&input.batch_id)?;
    let mut aws = resolution::aws_credentials(&input.optional_params);
    if kind == BedrockArn::AsyncInvoke && aws.region_name.is_none() {
        aws.region_name = Some(DEFAULT_BEDROCK_REGION.to_owned());
    }
    let batch_id = input.batch_id.clone();
    Some(match kind {
        BedrockArn::AsyncInvoke => {
            BatchProviderReadCall::BedrockAsyncInvoke(Box::new(BedrockAsyncInvokeReadCall {
                batch_id,
                aws,
            }))
        }
        BedrockArn::ModelInvocationJob => BatchProviderReadCall::BedrockModelInvocationJob(
            Box::new(BedrockModelInvocationJobReadCall { batch_id, aws }),
        ),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BedrockArn {
    AsyncInvoke,
    ModelInvocationJob,
}

impl BedrockArn {
    fn parse(batch_id: &str) -> Option<Self> {
        if !batch_id.starts_with("arn:aws") || !batch_id.contains(":bedrock:") {
            return None;
        }
        if batch_id.contains(":async-invoke/") {
            Some(Self::AsyncInvoke)
        } else if batch_id.contains(":model-invocation-job/") {
            Some(Self::ModelInvocationJob)
        } else {
            None
        }
    }
}

// a provider config comes before the legacy branches, and only bedrock and mistral declare one
pub(super) fn call(
    input: &BatchProviderReadInput,
    request: &RetrieveBatchRequest,
    timeout: Timeout,
    settings: &Settings,
) -> Result<BatchProviderReadCall, ReadCallError> {
    if let Some(model) = input.model.as_deref()
        && matches!(
            input.provider,
            BatchReadProvider::Bedrock | BatchReadProvider::Mistral
        )
    {
        return Ok(BatchProviderReadCall::ProviderConfig(Box::new(
            resolution::provider_config(input, model, timeout),
        )));
    }
    legacy_call(input, request, timeout, settings)
}

// the branches that read a provider's own endpoint once no provider config applies
fn legacy_call(
    input: &BatchProviderReadInput,
    request: &RetrieveBatchRequest,
    timeout: Timeout,
    settings: &Settings,
) -> Result<BatchProviderReadCall, ReadCallError> {
    let selected = match input.provider {
        BatchReadProvider::Xai => {
            BatchProviderReadCall::Xai(Box::new(resolution::xai(input, timeout)))
        }
        BatchReadProvider::OpenAi
        | BatchReadProvider::HostedVllm
        | BatchReadProvider::LitellmProxy => BatchProviderReadCall::OpenAiCompatible(Box::new(
            resolution::openai(input, request, settings, timeout),
        )),
        BatchReadProvider::Azure => BatchProviderReadCall::Azure(Box::new(resolution::azure(
            input, request, settings, timeout,
        ))),
        BatchReadProvider::VertexAi => {
            BatchProviderReadCall::VertexAi(Box::new(resolution::vertex(input, settings, timeout)?))
        }
        BatchReadProvider::Anthropic => BatchProviderReadCall::Anthropic(Box::new(
            resolution::anthropic(input, settings, timeout),
        )),
        BatchReadProvider::Bedrock | BatchReadProvider::Mistral | BatchReadProvider::Other(_) => {
            return Err(ReadCallError::Unsupported {
                provider: input.provider.name().to_owned(),
            });
        }
    };
    Ok(selected)
}
