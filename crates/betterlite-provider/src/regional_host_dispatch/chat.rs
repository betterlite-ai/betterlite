use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use betterlite_openai::minimax_chat;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::agent_platform_dispatch::context::headers_or_global;
use crate::hosted_platform_dispatch::handler::ChatCompletionHandler;

use super::call::completion_call;
use super::context::{CallLogging, DispatchContext, first_nonempty, first_secret};
use super::error::DispatchError;
use super::logging;

const NLP_CLOUD_DEFAULT_API_BASE: &str = "https://api.nlpcloud.io/v1/gpu/";

// LiteLLM's dispatch arms for the regional hosts; the skip keeps the long no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::GigaChat => Some(gigachat(context, handler, log)),
        ChatConfigTag::MinimaxChat => Some(minimax(context, handler, log)),
        ChatConfigTag::NlpCloud => Some(nlp_cloud(context, handler, log)),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat
        | ChatConfigTag::CodestralTextCompletion | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi | ChatConfigTag::GroqChat
        | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::Mistral | ChatConfigTag::ModelScopeChat
        | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius | ChatConfigTag::Novita | ChatConfigTag::Nscale
        | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat | ChatConfigTag::Ollama | ChatConfigTag::OllamaChat | ChatConfigTag::Oobabooga
        | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat
        | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat
        | ChatConfigTag::PerplexityChat | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat
        | ChatConfigTag::RagFlow | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova
        | ChatConfigTag::SailChat | ChatConfigTag::Sambanova | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// _complete_gigachat: the passed key, litellm.api_key, litellm.gigachat_key, GIGACHAT_API_KEY and GIGACHAT_CREDENTIALS; the passed headers, litellm.headers, then an empty map; the base as it came
fn gigachat<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = completion_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key,
        context.settings.gigachat_key,
        context.settings.env_gigachat_api_key,
        context.settings.env_gigachat_credentials,
    ]);
    let headers =
        headers_or_global(context.headers.cloned(), context.settings.headers).unwrap_or_default();
    call.headers = Some(headers.clone());
    match handler.complete(&call) {
        Ok(reply) => Ok(reply),
        Err(error) => {
            logging::post(
                log,
                &call.messages,
                LogJson::Text(error.to_string()),
                Some(&headers),
            )?;
            Err(DispatchError::Handler(error))
        }
    }
}

// _complete_minimax: the passed key, MINIMAX_API_KEY, then litellm.api_key; the passed base, litellm.api_base, MINIMAX_API_BASE, then the public default; the headers as they came
fn minimax<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = completion_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.env_minimax_api_key,
        context.settings.api_key,
    ]);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base,
            context.settings.env_minimax_api_base,
        ])
        .unwrap_or_else(|| String::from(minimax_chat::DEFAULT_API_BASE)),
    );
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    logging::post(log, &call.messages, LogJson::Null, None)?;
    Ok(reply)
}

// _complete_nlp_cloud: the passed key, nlp_cloud_key, NLP_CLOUD_API_KEY, then litellm.api_key; the passed base, litellm.api_base, NLP_CLOUD_API_BASE, then the GPU default
fn nlp_cloud<R>(
    context: &DispatchContext<'_>,
    handler: &(dyn ChatCompletionHandler<Reply = R> + Sync),
    log: &mut CallLogging<'_, '_>,
) -> Result<R, DispatchError> {
    let mut call = completion_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.nlp_cloud_key,
        context.settings.env_nlp_cloud_api_key,
        context.settings.api_key,
    ]);
    call.api_base = Some(
        first_nonempty([
            context.api_base,
            context.settings.api_base,
            context.settings.env_nlp_cloud_api_base,
        ])
        .unwrap_or_else(|| String::from(NLP_CLOUD_DEFAULT_API_BASE)),
    );
    call.headers = None;
    call.provider_config = None;
    call.stream = Some(streams(context.optional_params));
    let reply = handler.complete(&call).map_err(DispatchError::Handler)?;
    if call.stream == Some(true) || context.acompletion {
        logging::post(log, &call.messages, LogJson::Null, None)?;
    }
    Ok(reply)
}

// LiteLLM wraps the reply when optional_params carries stream true, and logs when it streams or the call is async
pub(super) fn streams(optional_params: &Object) -> bool {
    matches!(optional_params.get("stream"), Some(Json::Bool(true)))
}
