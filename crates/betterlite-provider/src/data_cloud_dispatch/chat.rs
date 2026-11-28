use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use crate::agent_platform_dispatch::context::headers_or_global;

use super::call::CompletionCall;
use super::context::DispatchContext;
use super::error::DispatchError;
use super::handler::{ChatCompletionHandler, HandlerError};
use super::logging;

const DATABRICKS_PROVIDER: &str = "databricks";

// LiteLLM's dispatch arms for the three cloud platforms; the skip keeps the long no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut LogContext<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::Databricks => Some(complete_databricks(context, handler, log)),
        ChatConfigTag::OciChat => Some(handler.complete(&passthrough(context)).map_err(DispatchError::Handler)),
        ChatConfigTag::Snowflake => Some(complete_cloud(context, &passthrough(context), false, handler, log)),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
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
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral
        | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius
        | ChatConfigTag::NlpCloud | ChatConfigTag::Novita | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::Ollama
        | ChatConfigTag::OllamaChat | ChatConfigTag::Oobabooga | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat
        | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat
        | ChatConfigTag::PerplexityChat | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat
        | ChatConfigTag::RagFlow | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova
        | ChatConfigTag::SailChat | ChatConfigTag::Sambanova | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// the handler call both completion arms share, then the failure record and the optional stream record
fn complete_cloud<R>(
    context: &DispatchContext<'_>,
    call: &CompletionCall,
    stream_record: bool,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut LogContext<'_, '_>,
) -> Result<R, DispatchError> {
    let reply = handler.complete(call);
    if let Err(error) = &reply {
        log_failure(log, context, call, error)?;
    }
    // LiteLLM logs the opened stream object itself; a Rust stream has no JSON form, so the record carries null
    if reply.is_ok() && stream_record && context.stream == Some(true) {
        logging::post(log, context.messages, Json::Null, call.headers.as_ref())?;
    }
    reply.map_err(DispatchError::Handler)
}

// _complete_databricks: the resolved call and the stream post_call on success
pub fn complete_databricks<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut LogContext<'_, '_>,
) -> Result<R, DispatchError> {
    match databricks_call(context) {
        Ok(call) => complete_cloud(context, &call, true, handler, log),
        Err(error) => Err(error),
    }
}

// the post_call record LiteLLM writes before the handler error goes back unchanged
fn log_failure(
    log: &mut LogContext<'_, '_>,
    context: &DispatchContext<'_>,
    call: &CompletionCall,
    error: &HandlerError,
) -> Result<(), DispatchError> {
    logging::post(
        log,
        context.messages,
        Json::Text(error.to_string()),
        call.headers.as_ref(),
    )
}

// _complete_snowflake and _complete_oci hand the context fields to the handler with no fallback
fn passthrough(context: &DispatchContext<'_>) -> CompletionCall {
    CompletionCall {
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
    }
}

// _complete_databricks' base and key chains, with the literal provider and no preselected config
fn databricks_call(context: &DispatchContext<'_>) -> Result<CompletionCall, DispatchError> {
    let base = context
        .api_base
        .filter(|value| !value.is_empty())
        .or(context.globals.api_base);
    let api_base = match base {
        Some(found) => Some(found.to_owned()),
        None => context.env.text("DATABRICKS_API_BASE")?,
    };
    let candidates = [
        context.api_key,
        context.globals.api_key,
        context.globals.databricks_key,
    ];
    let usable = candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty());
    let api_key = match usable {
        Some(key) => Some(key.clone()),
        None => context.env.secret("DATABRICKS_API_KEY")?,
    };
    Ok(CompletionCall {
        api_base,
        api_key,
        custom_llm_provider: DATABRICKS_PROVIDER.to_owned(),
        headers: headers_or_global(context.headers.cloned(), context.globals.headers),
        provider_config: None,
        ..passthrough(context)
    })
}
