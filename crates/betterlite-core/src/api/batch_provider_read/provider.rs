use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::batches::sdk::Timeout;

use super::call::{
    AwsCredentials, BatchProviderReadCall, BedrockAsyncInvokeReadCall, BedrockJobReadCall,
    XaiBatchReadCall,
};
use super::error::ReadCallError;
use super::resolution::{Resolution, provider_config};
use super::{BatchProviderReadInput, BatchReadProvider};

const DEFAULT_BEDROCK_REGION: &str = "us-east-1";

// batch ids that carry a Bedrock ARN are read from AWS rather than from a provider batch endpoint
pub(super) fn bedrock_arn_call(
    input: &BatchProviderReadInput,
    mut aws: AwsCredentials,
) -> Option<BatchProviderReadCall> {
    let kind = BedrockArn::parse(&input.batch_id)?;
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
        BedrockArn::ModelInvocationJob => {
            BatchProviderReadCall::BedrockModelInvocationJob(Box::new(BedrockJobReadCall {
                batch_id,
                aws,
            }))
        }
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

// bedrock and mistral read a provider config when a model is set; every other provider reads its own endpoint
pub(super) fn call(
    input: &BatchProviderReadInput,
    timeout: Timeout,
    settings: &Settings,
    aws: AwsCredentials,
) -> Result<BatchProviderReadCall, ReadCallError> {
    let resolution = Resolution::new(input, settings);
    let unsupported = ReadCallError::Unsupported {
        provider: input.provider.name().to_owned(),
    };
    let call = match input.provider {
        BatchReadProvider::Bedrock | BatchReadProvider::Mistral => {
            let model = input.model.as_deref().ok_or(unsupported)?;
            let config = provider_config(input, model, timeout, aws);
            BatchProviderReadCall::ProviderConfig(Box::new(config))
        }
        BatchReadProvider::Xai => BatchProviderReadCall::Xai(Box::new(XaiBatchReadCall {
            batch_id: input.batch_id.clone(),
            api_base: input.optional_params.api_base.clone(),
            api_key: input.optional_params.api_key.clone(),
            timeout,
            is_async: input.aretrieve_batch,
        })),
        BatchReadProvider::OpenAi
        | BatchReadProvider::HostedVllm
        | BatchReadProvider::LitellmProxy => {
            BatchProviderReadCall::OpenAiCompatible(Box::new(resolution.openai(timeout)))
        }
        BatchReadProvider::Azure => {
            BatchProviderReadCall::Azure(Box::new(resolution.azure(timeout)))
        }
        BatchReadProvider::VertexAi => {
            BatchProviderReadCall::VertexAi(Box::new(resolution.vertex(timeout)?))
        }
        BatchReadProvider::Anthropic => {
            BatchProviderReadCall::Anthropic(Box::new(resolution.anthropic(timeout)))
        }
        BatchReadProvider::Other(_) => return Err(unsupported),
    };
    Ok(call)
}
