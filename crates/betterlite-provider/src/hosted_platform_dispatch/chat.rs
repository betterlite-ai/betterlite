use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_openai::cloudflare_chat;
use betterlite_openai::fireworks_ai_chat_params::FireworksCapabilities;
use betterlite_openai::fireworks_ai_text_completion::extra_body::map_extra_body_params;
use betterlite_openai::groq_chat;
use betterlite_openai::openai_base_config::Object as ChatObject;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;

use crate::agent_platform_dispatch::context::headers_or_global;

use super::call::CompletionCall;
use super::context::{CallLogging, DispatchContext, first_nonempty, first_secret};
use super::error::DispatchError;
use super::handler::{ChatCompletionHandler, HandlerError};
use super::logging;

const CLOUDFLARE_PROVIDER: &str = "cloudflare";

// LiteLLM's dispatch arms for the hosted platforms; the skip keeps the long no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub async fn dispatch<P, R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    capabilities: &mut P,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Option<Result<R, DispatchError>>
where
    P: FireworksCapabilities + Send,
{
    match tag {
        ChatConfigTag::CloudflareChat => Some(cloudflare(context, handler)),
        ChatConfigTag::FireworksAi => Some(fireworks(context, capabilities, handler, log).await),
        ChatConfigTag::GroqChat => Some(groq(context, handler)),
        ChatConfigTag::TogetherAiChat => Some(together(context, handler, log)),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CodestralTextCompletion
        | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral
        | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius
        | ChatConfigTag::NlpCloud | ChatConfigTag::Novita | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat
        | ChatConfigTag::Ollama | ChatConfigTag::OllamaChat | ChatConfigTag::Oobabooga
        | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat
        | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat
        | ChatConfigTag::PerplexityChat | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat
        | ChatConfigTag::RagFlow | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova
        | ChatConfigTag::SailChat | ChatConfigTag::Sambanova | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// the call every arm shares, before it overrides the fields its own resolution changes
fn resolved(context: &DispatchContext<'_>) -> CompletionCall {
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

// _complete_fireworks_ai maps the optional params only when the Fireworks chat config is the provider config, then logs a failed call's error text with the headers
async fn fireworks<P, R>(
    context: &DispatchContext<'_>,
    capabilities: &mut P,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError>
where
    P: FireworksCapabilities + Send,
{
    let mut call = resolved(context);
    if call.provider_config == Some(ChatConfigTag::FireworksAi) {
        let params: ChatObject = logging::convert(context.optional_params)?;
        let mapped = map_extra_body_params(capabilities, context.model, &params).await;
        call.optional_params = logging::convert(&mapped)?;
    }
    logged(
        handler.complete(&call),
        context.messages,
        call.headers.as_ref(),
        log,
    )
}

// _complete_together_ai passes every field and logs a failed call the same way
fn together<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let call = resolved(context);
    logged(
        handler.complete(&call),
        context.messages,
        call.headers.as_ref(),
        log,
    )
}

// both arms log the error text as the original response and return the same error unchanged
fn logged<R>(
    reply: Result<R, HandlerError>,
    messages: &[Message],
    headers: Option<&BTreeMap<String, String>>,
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    match reply {
        Ok(value) => Ok(value),
        Err(error) => {
            logging::post(log, messages, &error, headers)?;
            Err(DispatchError::Handler(error))
        }
    }
}

// _complete_groq: the passed base, litellm.api_base, GROQ_API_BASE and the public default; the passed key, litellm.api_key, litellm.groq_key and GROQ_API_KEY; the passed headers, then litellm.headers
fn groq<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let mut call = resolved(context);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base,
            context.settings.env_groq_api_base,
        ])
        .unwrap_or_else(|| String::from(groq_chat::DEFAULT_API_BASE)),
    );
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key,
        context.settings.groq_key,
        context.settings.env_groq_api_key,
    ]);
    call.headers = headers_or_global(context.headers.cloned(), context.settings.headers);
    merge_config(&mut call.optional_params, context.groq_config);
    call.provider_config = None;
    handler.complete(&call).map_err(DispatchError::Handler)
}

// _complete_cloudflare: the passed key, litellm.cloudflare_api_key, litellm.api_key and CLOUDFLARE_API_KEY; the passed base, litellm.api_base and CLOUDFLARE_API_BASE, which the URL rule completes with the account id
fn cloudflare<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let mut call = resolved(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.cloudflare_api_key,
        context.settings.api_key,
        context.settings.env_cloudflare_api_key,
    ]);
    let base = first_nonempty([
        context.api_base,
        context.settings.api_base,
        context.settings.env_cloudflare_api_base,
    ]);
    call.api_base = Some(cloudflare_chat::complete_url(
        base.as_deref(),
        context.settings.env_cloudflare_account_id,
    )?);
    call.headers = context.headers.cloned();
    call.provider_config = None;
    CLOUDFLARE_PROVIDER.clone_into(&mut call.custom_llm_provider);
    handler.complete(&call).map_err(DispatchError::Handler)
}

// LiteLLM adds a GroqChatConfig.get_config key only where optional_params lacks it, so a request value wins
pub(super) fn merge_config(optional_params: &mut Object, config: &Object) {
    for (key, value) in config {
        optional_params
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}
