use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json as LogJson;
use betterlite_openai::vercel_ai_gateway::DEFAULT_API_BASE as VERCEL_DEFAULT_API_BASE;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::agent_platform_dispatch::context::headers_or_global;

use super::call::CompletionCall;
use super::context::{
    DispatchContext, DispatchSettings, first_nonempty, first_secret, referer_headers,
};
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;
use super::logging;

pub const OPENROUTER_PROVIDER: &str = "openrouter";
pub const VERCEL_PROVIDER: &str = "vercel_ai_gateway";
pub const OPENROUTER_DEFAULT_API_BASE: &str = "https://openrouter.ai/api/v1";
pub(super) const OPENROUTER_REFERER: &str = "HTTP-Referer";
pub(super) const OPENROUTER_TITLE: &str = "X-Title";

// LiteLLM's dispatch arms for the gateway providers; the skip keeps the exhaustive no-op arm within clippy's line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut LogContext<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::Openrouter => Some(complete(context, handler, log, &Gateway::OPENROUTER)),
        ChatConfigTag::VercelAiGateway => Some(complete(context, handler, log, &Gateway::VERCEL)),
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

// the names and the environment positions one arm uses, so both arms share one body
pub(super) struct Gateway {
    pub(super) which: Which,
    pub(super) provider: &'static str,
    pub(super) default_api_base: &'static str,
}

#[derive(Clone, Copy)]
pub(super) enum Which {
    Openrouter,
    Vercel,
}

impl Gateway {
    // _complete_openrouter: the base chain ends at OPENROUTER_API_BASE, the key chain at OR_API_KEY
    pub(super) const OPENROUTER: Self = Self {
        which: Which::Openrouter,
        provider: OPENROUTER_PROVIDER,
        default_api_base: OPENROUTER_DEFAULT_API_BASE,
    };

    // litellm.completion reaches Vercel through its openai-compatible path: no referer pair, no config merge
    pub(super) const VERCEL: Self = Self {
        which: Which::Vercel,
        provider: VERCEL_PROVIDER,
        default_api_base: VERCEL_DEFAULT_API_BASE,
    };
}

// the provider variables each arm falls back to, in chain order; Vercel's chat chain stops at its gateway key
pub(super) struct Chains<'settings> {
    pub(super) api_base: Option<&'settings str>,
    pub(super) first_key: Option<&'settings SecretString>,
    pub(super) second_key: Option<&'settings SecretString>,
    pub(super) third_key: Option<&'settings SecretString>,
    pub(super) site_url: Option<&'settings str>,
    pub(super) app_name: Option<&'settings str>,
}

pub(super) const fn chains(settings: DispatchSettings<'_>, which: Which) -> Chains<'_> {
    match which {
        Which::Openrouter => Chains {
            api_base: settings.openrouter_api_base,
            first_key: settings.openrouter_key,
            second_key: settings.openrouter_api_key,
            third_key: settings.or_api_key,
            site_url: settings.or_site_url,
            app_name: settings.or_app_name,
        },
        Which::Vercel => Chains {
            api_base: settings.vercel_ai_gateway_api_base,
            first_key: settings.vercel_ai_gateway_api_key,
            second_key: None,
            third_key: None,
            site_url: settings.vercel_site_url,
            app_name: settings.vercel_app_name,
        },
    }
}

fn complete<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut LogContext<'_, '_>,
    gateway: &Gateway,
) -> Result<R, DispatchError> {
    let values = chains(context.settings, gateway.which);
    let mut call = CompletionCall::from_context(context);
    call.api_base = Some(
        first_nonempty([context.api_base, context.settings.api_base, values.api_base])
            .unwrap_or_else(|| String::from(gateway.default_api_base)),
    );
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key,
        values.first_key,
        values.second_key,
        values.third_key,
    ]);
    call.custom_llm_provider = String::from(gateway.provider);
    call.headers = match gateway.which {
        Which::Openrouter => Some(referer_headers(
            context.headers,
            context.settings.headers,
            (OPENROUTER_REFERER, OPENROUTER_TITLE),
            values.site_url,
            values.app_name,
        )),
        Which::Vercel => headers_or_global(context.headers.cloned(), context.settings.headers),
    };
    if matches!(gateway.which, Which::Openrouter) {
        merge_config(
            &mut call.optional_params,
            context.class_config,
            gateway.provider,
        )?;
    }
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
