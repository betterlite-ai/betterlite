use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;

use crate::ragflow_chat::env::{CredentialsInput, Globals, PROVIDER, provider_info};

use self::call::CompletionCall;
use self::context::DispatchContext;
use self::error::DispatchError;
use self::handler::{ChatCompletionHandler, HandlerError};

pub mod call;
pub mod context;
pub mod error;
pub mod handler;
pub mod logging;

mod tests;

// LiteLLM's _complete_ragflow arm; the skip keeps the long no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut LogContext<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::RagFlow => Some(complete_ragflow(context, handler, log)),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat
        | ChatConfigTag::CodestralTextCompletion | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::GroqChat | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral
        | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius
        | ChatConfigTag::NlpCloud | ChatConfigTag::Novita | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat | ChatConfigTag::Ollama
        | ChatConfigTag::OllamaChat | ChatConfigTag::Oobabooga | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat
        | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat
        | ChatConfigTag::PerplexityChat | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat
        | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova
        | ChatConfigTag::SailChat | ChatConfigTag::Sambanova | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// _complete_ragflow: the provider info pair, the caller's headers and provider_config, then the failure record
fn complete_ragflow<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut LogContext<'_, '_>,
) -> Result<R, DispatchError> {
    let info = provider_info(&CredentialsInput {
        model: context.model,
        api_base: context.api_base,
        api_key: context.api_key,
        globals: Globals {
            api_base: context.globals.api_base,
            api_key: context.globals.api_key,
        },
        secrets: context.secrets,
        environment: context.environment,
    })?;
    let call = CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: info.api_base,
        api_key: info.api_key,
        custom_llm_provider: String::from(PROVIDER),
        // the ragflow arm passes the caller's headers and config through and never merges litellm.headers
        headers: context.headers.cloned(),
        litellm_params: context.litellm_params.clone(),
        optional_params: context.optional_params.clone(),
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: context.provider_config,
    };
    match handler.complete(&call) {
        Ok(reply) => Ok(reply),
        Err(error) => {
            log_failure(log, context.messages, &call, &error)?;
            Err(DispatchError::Handler(error))
        }
    }
}

fn log_failure(
    log: &mut LogContext<'_, '_>,
    messages: &[Message],
    call: &CompletionCall,
    error: &HandlerError,
) -> Result<(), DispatchError> {
    logging::post(
        log,
        messages,
        Json::Text(error.to_string()),
        call.headers.as_ref(),
    )
}
