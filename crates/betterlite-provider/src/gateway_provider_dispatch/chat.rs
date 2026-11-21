use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use betterlite_openai::vercel_ai_gateway::DEFAULT_API_BASE as VERCEL_DEFAULT_API_BASE;
use betterlite_prompts::arguments::json::{Json, Object};

use super::call::CompletionCall;
use super::context::{CallLogging, DispatchContext, first_nonempty, first_secret, referer_headers};
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;
use super::logging;

pub const OPENROUTER_PROVIDER: &str = "openrouter";
pub const VERCEL_PROVIDER: &str = "vercel_ai_gateway";
pub const OPENROUTER_DEFAULT_API_BASE: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_REFERER: &str = "HTTP-Referer";
const OPENROUTER_TITLE: &str = "X-Title";
const VERCEL_REFERER: &str = "http-referer";
const VERCEL_TITLE: &str = "x-title";

// LiteLLM's dispatch arms for the gateway providers; the skip keeps the exhaustive no-op arm within clippy's line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::Openrouter => Some(openrouter(context, handler, log)),
        ChatConfigTag::VercelAiGateway => Some(vercel(context, handler, log)),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat | ChatConfigTag::CodestralTextCompletion
        | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::GroqChat | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral
        | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius
        | ChatConfigTag::NlpCloud | ChatConfigTag::Novita | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat
        | ChatConfigTag::Ollama | ChatConfigTag::OllamaChat | ChatConfigTag::Oobabooga
        | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat
        | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::OvhCloudChat
        | ChatConfigTag::PerplexityChat | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat
        | ChatConfigTag::RagFlow | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova
        | ChatConfigTag::SailChat | ChatConfigTag::Sambanova | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat
        | ChatConfigTag::Triton | ChatConfigTag::V0Chat | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// _complete_openrouter: the passed base, litellm.api_base, OPENROUTER_API_BASE and the public default; the passed key, litellm.api_key, litellm.openrouter_key, OPENROUTER_API_KEY and OR_API_KEY; then the referer headers and the class config
fn openrouter<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = CompletionCall::from_context(context);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base,
            context.settings.openrouter_api_base,
        ])
        .unwrap_or_else(|| String::from(OPENROUTER_DEFAULT_API_BASE)),
    );
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key,
        context.settings.openrouter_key,
        context.settings.openrouter_api_key,
        context.settings.or_api_key,
    ]);
    call.custom_llm_provider = String::from(OPENROUTER_PROVIDER);
    call.headers = Some(referer_headers(
        context.headers,
        context.settings.headers,
        OPENROUTER_REFERER,
        OPENROUTER_TITLE,
        context.settings.or_site_url,
        context.settings.or_app_name,
    ));
    merge_config(
        &mut call.optional_params,
        context.class_config,
        OPENROUTER_PROVIDER,
    )?;
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    logging::post(log, context.messages, LogJson::Null)?;
    Ok(reply)
}

// _complete_vercel_ai_gateway: the same shape in lower case; the chat chain never reads VERCEL_OIDC_TOKEN
fn vercel<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = CompletionCall::from_context(context);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base,
            context.settings.vercel_ai_gateway_api_base,
        ])
        .unwrap_or_else(|| String::from(VERCEL_DEFAULT_API_BASE)),
    );
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key,
        context.settings.vercel_ai_gateway_api_key,
    ]);
    call.custom_llm_provider = String::from(VERCEL_PROVIDER);
    call.headers = Some(referer_headers(
        context.headers,
        context.settings.headers,
        VERCEL_REFERER,
        VERCEL_TITLE,
        context.settings.vercel_site_url,
        context.settings.vercel_app_name,
    ));
    merge_config(
        &mut call.optional_params,
        context.class_config,
        VERCEL_PROVIDER,
    )?;
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    logging::post(log, context.messages, LogJson::Null)?;
    Ok(reply)
}

// extra_body is updated key by key, or set when the request has none; every other key is added only when optional_params lacks it
pub(super) fn merge_config(
    optional_params: &mut Object,
    config: &Object,
    provider: &'static str,
) -> Result<(), DispatchError> {
    for (key, value) in config {
        if key == "extra_body" {
            let Json::Object(extra) = value else {
                return Err(DispatchError::ExtraBody { provider });
            };
            match optional_params
                .entry(key.clone())
                .or_insert_with(|| Json::Object(Object::new()))
            {
                Json::Object(existing) => existing.extend(extra.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    return Err(DispatchError::ExtraBody { provider });
                }
            }
            continue;
        }
        if !optional_params.contains_key(key) {
            optional_params.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}
