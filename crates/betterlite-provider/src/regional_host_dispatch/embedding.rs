use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::dashscope_family::{self, DashscopeFamily};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::hosted_platform_dispatch::call::EmbeddingCall;

use super::call::embedding_call;
use super::context::{EmbeddingContext, first_secret};
use super::error::DispatchError;

const VOLCENGINE_MISSING_KEY: &str = "Missing API key for Volcengine. Set ARK_API_KEY or VOLCENGINE_API_KEY environment variable or pass api_key parameter.";

// main.py's embedding arms: volcengine, the three DashScope brands and gigachat resolve their key here; the skip keeps the exhaustive no-op arm within clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch(
    provider: ProviderId,
    context: &EmbeddingContext<'_>,
) -> Option<Result<EmbeddingCall, DispatchError>> {
    match provider {
        ProviderId::Volcengine => Some(volcengine(context)),
        ProviderId::Dashscope => Some(family(DashscopeFamily::Dashscope, context)),
        ProviderId::Qwencloud => Some(family(DashscopeFamily::QwenCloud, context)),
        ProviderId::QwenAiPlatform => Some(family(DashscopeFamily::QwenAiPlatform, context)),
        ProviderId::Gigachat => Some(Ok(gigachat(context))),
        ProviderId::Openai | ProviderId::Chatgpt | ProviderId::OpenaiLike | ProviderId::JinaAi | ProviderId::Xai | ProviderId::Zai | ProviderId::CustomOpenai | ProviderId::TextCompletionOpenai | ProviderId::Cohere | ProviderId::CohereChat | ProviderId::Clarifai | ProviderId::Anthropic
        | ProviderId::AnthropicText | ProviderId::Bytez | ProviderId::Replicate | ProviderId::Reducto | ProviderId::AwsTextract | ProviderId::Runwayml | ProviderId::AwsPolly | ProviderId::Transcribe | ProviderId::Huggingface | ProviderId::TogetherAi | ProviderId::Openrouter | ProviderId::Datarobot
        | ProviderId::VertexAi | ProviderId::VertexAiBeta | ProviderId::Gemini | ProviderId::Ai21 | ProviderId::Baseten | ProviderId::BlackForestLabs | ProviderId::Azure | ProviderId::AzureText | ProviderId::AzureAi | ProviderId::Sagemaker | ProviderId::SagemakerChat | ProviderId::SagemakerNova
        | ProviderId::Bedrock | ProviderId::Vllm | ProviderId::NlpCloud | ProviderId::Petals | ProviderId::Ollama | ProviderId::OllamaChat | ProviderId::Oobabooga | ProviderId::Triton | ProviderId::HostedVllm | ProviderId::Llamafile | ProviderId::LmStudio | ProviderId::Xinference | ProviderId::Deepinfra | ProviderId::Typesafe | ProviderId::StrandsDecider | ProviderId::Mistral | ProviderId::Milvus | ProviderId::Groq
        | ProviderId::A2a | ProviderId::NvidiaNim | ProviderId::NvidiaRiva | ProviderId::Soniox | ProviderId::Cerebras | ProviderId::Nadir | ProviderId::Ai21Chat | ProviderId::Codestral | ProviderId::TextCompletionCodestral | ProviderId::Modelscope | ProviderId::Moonshot | ProviderId::Publicai | ProviderId::V0 | ProviderId::Morph | ProviderId::LambdaAi | ProviderId::Inception | ProviderId::TextCompletionInception | ProviderId::Deepseek | ProviderId::Sambanova
        | ProviderId::Maritalk | ProviderId::Voyage | ProviderId::Cloudflare | ProviderId::Friendliai | ProviderId::FeatherlessAi | ProviderId::Watsonx | ProviderId::WatsonxText | ProviderId::Predibase | ProviderId::Databricks | ProviderId::Empower | ProviderId::Github
        | ProviderId::Ragflow | ProviderId::Compactifai | ProviderId::DockerModelRunner | ProviderId::Custom | ProviderId::LitellmProxy | ProviderId::Tencent | ProviderId::Galadriel | ProviderId::Nebius | ProviderId::Infinity | ProviderId::Deepgram | ProviderId::Elevenlabs | ProviderId::Novita
        | ProviderId::AiohttpOpenai | ProviderId::Langfuse | ProviderId::Humanloop | ProviderId::Topaz | ProviderId::SapGenerativeAiHub | ProviderId::Assemblyai | ProviderId::AzureSpeech | ProviderId::CharityEngine | ProviderId::GithubCopilot | ProviderId::Snowflake | ProviderId::GradientAi | ProviderId::Llama
        | ProviderId::Nscale | ProviderId::PgVector | ProviderId::S3Vectors | ProviderId::Valkey | ProviderId::Mongodb | ProviderId::Helicone | ProviderId::Hyperbolic | ProviderId::Recraft | ProviderId::FalAi | ProviderId::Stability | ProviderId::Heroku | ProviderId::Aiml
        | ProviderId::Cometapi | ProviderId::Oci | ProviderId::AutoRouter | ProviderId::VercelAiGateway | ProviderId::Edenai | ProviderId::Dotprompt | ProviderId::Manus | ProviderId::Wandb | ProviderId::Ovhcloud | ProviderId::Scaleway | ProviderId::Lemonade | ProviderId::AmazonNova
        | ProviderId::A2aAgent | ProviderId::Langgraph | ProviderId::Langflow | ProviderId::Minimax | ProviderId::Synthetic | ProviderId::Apertis | ProviderId::Nanogpt | ProviderId::Poe | ProviderId::Chutes | ProviderId::Abliteration | ProviderId::Llamagate | ProviderId::Gmi
        | ProviderId::Sarvam | ProviderId::Aihubmix | ProviderId::Crusoe | ProviderId::Empiriolabs | ProviderId::Veniceai | ProviderId::Neosantara | ProviderId::Parasail | ProviderId::XiaomiMimo | ProviderId::Tensormesh | ProviderId::Libertai | ProviderId::Pinstripes | ProviderId::Cognition
        | ProviderId::Cortecs | ProviderId::ScxAi | ProviderId::Prism | ProviderId::Darkbloom | ProviderId::Meta | ProviderId::Sail | ProviderId::LitellmAgent | ProviderId::Cursor | ProviderId::BedrockMantle | ProviderId::Gdc => None,
    }
}

// the volcengine arm: the passed key, litellm.api_key, ARK_API_KEY, VOLCENGINE_API_KEY; extra_headers or an empty map; empty litellm_params
fn volcengine(context: &EmbeddingContext<'_>) -> Result<EmbeddingCall, DispatchError> {
    let mut call = embedding_call(context);
    call.api_key = Some(
        first_secret([
            context.api_key,
            context.settings.api_key,
            context.settings.env_ark_api_key,
            context.settings.env_volcengine_api_key,
        ])
        .ok_or(DispatchError::MissingKey {
            message: VOLCENGINE_MISSING_KEY,
        })?,
    );
    call.headers = Some(wire_headers(context));
    call.litellm_params = Object::new();
    Ok(call)
}

// the three DashScope brands read the passed key, litellm.api_key, their own env key and DASHSCOPE_API_KEY, and name the brand they were called as when none is set
fn family(
    brand: DashscopeFamily,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingCall, DispatchError> {
    let key = dashscope_family::resolve_family_api_key(
        brand,
        context.api_key.map(|key| key.expose_secret().as_str()),
        brand_key(brand, context),
        context.settings.env_dashscope_api_key,
    )
    .ok_or(DispatchError::MissingKey {
        message: dashscope_family::missing_family_key_message(brand),
    })?;
    let mut call = embedding_call(context);
    call.api_key = Some(key);
    call.headers = Some(wire_headers(context));
    call.litellm_params = Object::new();
    Ok(call)
}

const fn brand_key<'request>(
    brand: DashscopeFamily,
    context: &EmbeddingContext<'request>,
) -> Option<&'request SecretString> {
    match brand {
        DashscopeFamily::Dashscope => None,
        DashscopeFamily::QwenCloud => context.settings.env_qwencloud_key,
        DashscopeFamily::QwenAiPlatform => context.settings.env_qwen_ai_platform_key,
    }
}

// the gigachat arm: the passed key, litellm.api_key, gigachat_key, GIGACHAT_CREDENTIALS, GIGACHAT_API_KEY; litellm_params holds only ssl_verify
fn gigachat(context: &EmbeddingContext<'_>) -> EmbeddingCall {
    let mut call = embedding_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.api_key,
        context.settings.gigachat_key,
        context.settings.env_gigachat_credentials,
        context.settings.env_gigachat_api_key,
    ]);
    call.headers = None;
    call.litellm_params = Object::from_iter([(
        String::from("ssl_verify"),
        context.ssl_verify.map_or(Json::Null, Json::Bool),
    )]);
    call
}

// the request headers the two arms hand the handler: extra_headers when it is there, else an empty map
fn wire_headers(context: &EmbeddingContext<'_>) -> BTreeMap<String, String> {
    context.extra_headers.cloned().unwrap_or_default()
}
