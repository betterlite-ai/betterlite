use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use secrecy::ExposeSecret as _;

use crate::agent_platform_dispatch::context::headers_or_global;

use super::call::CompletionCall;
use super::context::{CallLogging, DispatchContext, environment_secret, environment_text};
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;
use super::logging;

const DATABRICKS_PROVIDER: &str = "databricks";

// LiteLLM's dispatch arms for the three cloud platforms; the skip keeps the 110-variant no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::Databricks => Some(complete_databricks(context, handler, log)),
        ChatConfigTag::OciChat => Some(complete_oci(context, handler)),
        ChatConfigTag::Snowflake => Some(complete_snowflake(context, handler, log)),
        ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat
        | ChatConfigTag::CodestralTextCompletion | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::GroqChat | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat
        | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat
        | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius | ChatConfigTag::NlpCloud | ChatConfigTag::Novita
        | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::Ollama | ChatConfigTag::OllamaChat
        | ChatConfigTag::Oobabooga | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat | ChatConfigTag::OpenAiOSeries
        | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat | ChatConfigTag::PerplexityChat
        | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat | ChatConfigTag::RagFlow
        | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova | ChatConfigTag::SailChat
        | ChatConfigTag::Sambanova | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// _complete_databricks: the context value, the global setting, then the environment, for base, key and headers
pub fn complete_databricks<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut api_base = context
        .api_base
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if api_base.is_none() {
        api_base = context.globals.api_base.map(str::to_owned);
    }
    if api_base.is_none() {
        api_base = environment_text(context.secrets, context.environment, "DATABRICKS_API_BASE")?;
    }
    let mut api_key = context
        .api_key
        .filter(|value| !value.expose_secret().is_empty())
        .cloned();
    if api_key.is_none() {
        api_key = context
            .globals
            .api_key
            .filter(|value| !value.expose_secret().is_empty())
            .cloned();
    }
    if api_key.is_none() {
        api_key = context
            .globals
            .databricks_key
            .filter(|value| !value.expose_secret().is_empty())
            .cloned();
    }
    if api_key.is_none() {
        api_key = environment_secret(context.secrets, context.environment, "DATABRICKS_API_KEY")?;
    }
    let headers = headers_or_global(context.headers.cloned(), context.globals.headers);
    let call = CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base,
        api_key,
        custom_llm_provider: DATABRICKS_PROVIDER.to_owned(),
        headers,
        litellm_params: context.litellm_params.clone(),
        optional_params: context.optional_params.clone(),
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: None,
    };
    match handler.complete(&call) {
        Ok(reply) => {
            if context.stream == Some(true) {
                logging::post_opened_stream(log, context.messages, call.headers.as_ref())?;
            }
            Ok(reply)
        }
        Err(error) => {
            logging::post_failure(
                log,
                context.messages,
                &error.to_string(),
                call.headers.as_ref(),
            )?;
            Err(DispatchError::Handler(error))
        }
    }
}

// _complete_snowflake resolves nothing itself: the key and base come from get_llm_provider in B211
pub fn complete_snowflake<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let call = CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: context.api_base.map(str::to_owned),
        api_key: context.api_key.cloned(),
        custom_llm_provider: context.custom_llm_provider.to_owned(),
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
            logging::post_failure(
                log,
                context.messages,
                &error.to_string(),
                call.headers.as_ref(),
            )?;
            Err(DispatchError::Handler(error))
        }
    }
}

// _complete_oci passes the context fields to the handler with no fallback and logs nothing
pub fn complete_oci<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let call = CompletionCall {
        model: context.model.to_owned(),
        stream: context.stream,
        acompletion: context.acompletion,
        api_base: context.api_base.map(str::to_owned),
        api_key: context.api_key.cloned(),
        custom_llm_provider: context.custom_llm_provider.to_owned(),
        headers: context.headers.cloned(),
        litellm_params: context.litellm_params.clone(),
        optional_params: context.optional_params.clone(),
        messages: context.messages.to_vec(),
        timeout: context.timeout,
        provider_config: context.provider_config,
    };
    Ok(handler.complete(&call)?)
}
