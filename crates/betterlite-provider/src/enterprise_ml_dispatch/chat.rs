use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::builtin::Message as BuiltinMessage;
use betterlite_prompts::message_content::Message;

use crate::agent_platform_dispatch::context::headers_or_global;
use crate::data_cloud_dispatch::handler::ChatCompletionHandler;

use super::call::{convert, signing_params};
use super::context::{ChatContext, ReplacedCall, handler_call, passthrough};
use super::error::EnterpriseError;
use super::sagemaker::{self, SagemakerCompletion, SagemakerDeps, SagemakerReply};
use super::watsonx;

// GenAIHubOrchestrationConfig's only class setting; get_config filters the None ones out
pub const SAP_MODEL_VERSION: &str = "latest";

static NO_PROMPTS: BTreeMap<String, CustomPromptEntry> = BTreeMap::new();

pub enum Completion<R> {
    Handler(R),
    Sagemaker(SagemakerReply),
}

// LiteLLM's dispatch arms for the three enterprise ML platforms; the skip keeps the long no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &ChatContext<'_>,
    deps: &SagemakerDeps<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Option<Result<Completion<R>, EnterpriseError>> {
    match tag {
        ChatConfigTag::GenAiHubOrchestration => Some(sap_chat(context, handler)),
        ChatConfigTag::IbmWatsonXChat => Some(watsonx_chat(context, handler)),
        ChatConfigTag::IbmWatsonXAi => Some(watsonx_text(context, handler)),
        ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova => Some(sagemaker_chat(context, handler)),
        ChatConfigTag::Sagemaker => Some(sagemaker_completion(context, deps).map(Completion::Sagemaker)),
        ChatConfigTag::A2a | ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat
        | ChatConfigTag::CodestralTextCompletion | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::GroqChat | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LangFlow | ChatConfigTag::LangGraph | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi
        | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral
        | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius
        | ChatConfigTag::NlpCloud | ChatConfigTag::Novita | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat
        | ChatConfigTag::Ollama | ChatConfigTag::OllamaChat | ChatConfigTag::Oobabooga | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5
        | ChatConfigTag::OpenAiLikeChat | ChatConfigTag::OpenAiOSeries | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter
        | ChatConfigTag::OvhCloudChat | ChatConfigTag::PerplexityChat | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat
        | ChatConfigTag::QwenCloudChat | ChatConfigTag::RagFlow | ChatConfigTag::Replicate | ChatConfigTag::SailChat | ChatConfigTag::Sambanova
        | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}

// _complete_sap: model_version merged into the params the caller left unset, headers falling back to litellm.headers
fn sap_chat<R>(
    context: &ChatContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<Completion<R>, EnterpriseError> {
    let base = &context.base;
    let mut optional_params = base.optional_params.clone();
    if !optional_params.contains_key("model_version") {
        optional_params.insert(
            String::from("model_version"),
            Json::Text(String::from(SAP_MODEL_VERSION)),
        );
    }
    let call = handler_call(
        base,
        ReplacedCall {
            optional_params,
            api_key: base.api_key.cloned(),
            api_base: base.api_base.map(str::to_owned),
            provider: base.custom_llm_provider,
            headers: headers_or_global(base.headers.cloned(), base.globals.headers),
        },
    );
    Ok(Completion::Handler(handler.complete(&call)?))
}

// _complete_watsonx hands every context field to the chat handler under the literal provider name
fn watsonx_chat<R>(
    context: &ChatContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<Completion<R>, EnterpriseError> {
    Ok(Completion::Handler(
        handler.complete(&passthrough(&context.base, "watsonx"))?,
    ))
}

// _complete_watsonx_text resolves the credentials first, then hands the call over under watsonx_text
fn watsonx_text<R>(
    context: &ChatContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<Completion<R>, EnterpriseError> {
    Ok(Completion::Handler(
        handler.complete(&watsonx::text_call(context)?)?,
    ))
}

// _complete_sagemaker_chat: the context provider keeps sagemaker_chat and sagemaker_nova apart
fn sagemaker_chat<R>(
    context: &ChatContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Result<Completion<R>, EnterpriseError> {
    let call = passthrough(&context.base, context.base.custom_llm_provider);
    Ok(Completion::Handler(handler.complete(&call)?))
}

// _complete_sagemaker: no key and no base, as the AWS values travel in optional params
fn sagemaker_completion(
    context: &ChatContext<'_>,
    deps: &SagemakerDeps<'_>,
) -> Result<SagemakerReply, EnterpriseError> {
    let base = &context.base;
    let messages = builtin_messages(base.messages)?;
    let optional_params: Options = convert(base.optional_params)?;
    let signing = signing_params(base.optional_params)?;
    let stream = matches!(base.optional_params.get("stream"), Some(Json::Bool(true)));
    let call = SagemakerCompletion {
        model: base.model,
        messages: &messages,
        optional_params: &optional_params,
        signing: &signing,
        custom_prompt_dict: context.sagemaker.custom_prompt_dict.unwrap_or(&NO_PROMPTS),
        global_custom_prompt_dict: context
            .sagemaker
            .global_custom_prompt_dict
            .unwrap_or(&NO_PROMPTS),
        hf_model_name: context.sagemaker.hf_model_name,
        stream,
    };
    sagemaker::complete(deps, &call)
}

// the context messages as B5960's prompt renderer reads them, with an absent content made explicit
fn builtin_messages(messages: &[Message]) -> Result<Vec<BuiltinMessage>, EnterpriseError> {
    messages
        .iter()
        .map(|message| {
            let mut value = serde_json::to_value(message)
                .map_err(|source| EnterpriseError::Encode { source })?;
            // an absent content becomes the null the prompt renderer reads, as the message dict carries no key
            let null =
                serde_json::to_value(()).map_err(|source| EnterpriseError::Encode { source })?;
            if let Some(fields) = value.as_object_mut() {
                fields.entry(String::from("content")).or_insert(null);
            }
            serde_json::from_value(value).map_err(|source| EnterpriseError::Decode { source })
        })
        .collect()
}
