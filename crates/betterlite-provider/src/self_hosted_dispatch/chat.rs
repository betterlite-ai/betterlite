use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use betterlite_models::lemonade_model_metadata::DEFAULT_API_KEY as LEMONADE_DEFAULT_API_KEY;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::petals::error::{ENGINE_MISSING_MESSAGE, ENGINE_STATUS, PetalsSetupError};
use crate::vllm::completion::ENGINE_ERROR;

use super::call::{CompletionCall, SelfHostedConfig};
use super::context::{CallLogging, DispatchContext, first_nonempty, first_secret, ollama_api_base};
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;
use super::logging;

const OOBABOOGA_PROVIDER: &str = "oobabooga";
const OLLAMA_PROVIDER: &str = "ollama";
const OLLAMA_CHAT_PROVIDER: &str = "ollama_chat";
const PETALS_PROVIDER: &str = "petals";
const AUTHORIZATION: &str = "Authorization";

// LiteLLM's dispatch arms for the self-hosted providers; the skip keeps the long no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::HostedVllmChat => Some(complete_hosted_vllm(context, handler, log)),
        ChatConfigTag::Oobabooga => Some(complete_oobabooga(context, handler)),
        ChatConfigTag::Ollama => Some(complete_ollama(context, handler)),
        ChatConfigTag::OllamaChat => Some(complete_ollama_chat(context, handler)),
        ChatConfigTag::Petals => Some(complete_petals(context, handler)),
        ChatConfigTag::Triton => Some(complete_triton(context, handler)),
        ChatConfigTag::Vllm => Some(Err(DispatchError::Vllm(ENGINE_ERROR))),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat
        | ChatConfigTag::CodestralTextCompletion | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::GroqChat | ChatConfigTag::HerokuChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral
        | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius
        | ChatConfigTag::NlpCloud | ChatConfigTag::Novita | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat
        | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat
        | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat
        | ChatConfigTag::PerplexityChat | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat
        | ChatConfigTag::RagFlow | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova
        | ChatConfigTag::SailChat | ChatConfigTag::Sambanova | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// the call every arm shares, before it overrides the fields its own resolution changes
fn resolved(context: &DispatchContext<'_>, config: SelfHostedConfig) -> CompletionCall {
    CompletionCall {
        config,
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
    }
}

// _complete_hosted_vllm: get_llm_provider resolves HOSTED_VLLM_API_BASE before the arm's chain, so the environment base wins over litellm.api_base; the key is the passed key, then HOSTED_VLLM_API_KEY
pub fn complete_hosted_vllm<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let call = CompletionCall {
        api_base: first_nonempty([
            context.api_base,
            context.settings.hosted_vllm_api_base,
            context.settings.api_base,
        ]),
        api_key: first_secret([context.api_key, context.settings.hosted_vllm_api_key]),
        ..resolved(context, SelfHostedConfig::HostedVllm)
    };
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    // main.py logs the provider response; the handler's reply has no JSON form at this seam (docs/deviations/B8157.yaml)
    logging::post(log, context.messages, LogJson::Null)?;
    Ok(reply)
}

// _complete_oobabooga: the base and the context's own provider string, with no key and no headers
pub fn complete_oobabooga<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let call = CompletionCall {
        api_key: None,
        headers: None,
        custom_llm_provider: OOBABOOGA_PROVIDER.to_owned(),
        ..resolved(context, SelfHostedConfig::Oobabooga)
    };
    handler.complete(&call).map_err(DispatchError::Handler)
}

// _complete_vllm runs inside the process, so B8153's engine error goes back without a handler call

// _complete_ollama: litellm.api_base, the passed base, OLLAMA_API_BASE, then the local server, and the bearer header
pub fn complete_ollama<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let api_key = first_secret([context.api_key, context.settings.ollama_api_key]);
    let headers = authorized_headers(context.headers, api_key.as_ref());
    let call = CompletionCall {
        api_base: Some(ollama_api_base(context.settings, context.api_base)),
        api_key,
        headers,
        custom_llm_provider: OLLAMA_PROVIDER.to_owned(),
        ..resolved(context, SelfHostedConfig::Ollama)
    };
    handler.complete(&call).map_err(DispatchError::Handler)
}

// _complete_ollama_chat: the same base chain, and the key chain continues through OLLAMA_API_KEY and litellm.api_key
pub fn complete_ollama_chat<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let api_key = first_secret([
        context.api_key,
        context.settings.ollama_key,
        context.settings.ollama_api_key,
        context.settings.api_key,
    ]);
    let headers = authorized_headers(context.headers, api_key.as_ref());
    let call = CompletionCall {
        api_base: Some(ollama_api_base(context.settings, context.api_base)),
        api_key,
        headers,
        custom_llm_provider: OLLAMA_CHAT_PROVIDER.to_owned(),
        ..resolved(context, SelfHostedConfig::OllamaChat)
    };
    handler.complete(&call).map_err(DispatchError::Handler)
}

// _complete_triton: litellm.api_base first, then the passed base, with the context's provider string
pub fn complete_triton<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let call = CompletionCall {
        api_base: first_nonempty([context.settings.api_base, context.api_base]),
        ..resolved(context, SelfHostedConfig::Triton)
    };
    handler.complete(&call).map_err(DispatchError::Handler)
}

// _complete_petals: the passed base, then litellm.api_base, with stream popped out of optional_params and neither the caller's headers nor a key in the call
pub fn complete_petals<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let api_base = first_nonempty([context.api_base, context.settings.api_base]);
    let (stream, optional_params) = take_stream(context.optional_params)?;
    let call = CompletionCall {
        api_base: api_base.clone(),
        stream,
        optional_params,
        api_key: None,
        headers: None,
        custom_llm_provider: PETALS_PROVIDER.to_owned(),
        ..resolved(context, SelfHostedConfig::Petals)
    };
    match api_base {
        Some(_) => handler.complete(&call).map_err(DispatchError::Handler),
        None => Err(DispatchError::Petals(PetalsSetupError::Engine {
            status_code: ENGINE_STATUS,
            message: ENGINE_MISSING_MESSAGE,
        })),
    }
}

// _complete_lemonade: get_llm_provider settles the key against the passed base first, so an explicit base without a key sends the default sentinel; otherwise the passed key, litellm.lemonade_key and LEMONADE_API_KEY decide
pub fn complete_lemonade<R>(
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<R, DispatchError> {
    let key_allowed = context.api_base.is_none()
        || context
            .api_key
            .is_some_and(|key| !key.expose_secret().is_empty());
    let api_key = if key_allowed {
        first_secret([
            context.api_key,
            context.settings.lemonade_key,
            context.settings.lemonade_api_key,
        ])
        .unwrap_or_else(|| SecretString::from(LEMONADE_DEFAULT_API_KEY))
    } else {
        SecretString::from(LEMONADE_DEFAULT_API_KEY)
    };
    let call = CompletionCall {
        api_key: Some(api_key),
        ..resolved(context, SelfHostedConfig::Lemonade)
    };
    handler.complete(&call).map_err(DispatchError::Handler)
}

// Python's optional_params.pop("stream", False): the parameter leaves the request and becomes the call's stream flag
fn take_stream(optional_params: &Object) -> Result<(Option<bool>, Object), DispatchError> {
    let mut params = optional_params.clone();
    let stream = match params.shift_remove("stream") {
        None | Some(Json::Null) => None,
        Some(Json::Bool(flag)) => Some(flag),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(DispatchError::StreamFlag);
        }
    };
    Ok((stream, params))
}

// the ollama arms add the bearer header themselves when the caller did not spell Authorization
fn authorized_headers(
    given: Option<&BTreeMap<String, String>>,
    key: Option<&SecretString>,
) -> Option<BTreeMap<String, String>> {
    let Some(key) = key else {
        return given.cloned();
    };
    let mut headers = given.cloned().unwrap_or_default();
    if !headers.contains_key(AUTHORIZATION) {
        headers.insert(
            AUTHORIZATION.to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    Some(headers)
}
