use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::dashscope_family::{self, DashscopeFamily};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret, SecretString};

use crate::hosted_platform_dispatch::call::EmbeddingCall;

use super::context::{EmbeddingContext, first_secret};
use super::embedding_volcengine::volcengine;
use super::error::DispatchError;

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
        | ProviderId::AiohttpOpenai | ProviderId::Langfuse | ProviderId::Humanloop | ProviderId::Topaz | ProviderId::SapGenerativeAiHub | ProviderId::Assemblyai | ProviderId::AzureSpeech | ProviderId::CharityEngine | ProviderId::GithubCopilot | ProviderId::Snowflake | ProviderId::GradientAi | ProviderId::Llama | ProviderId::FireworksAi | ProviderId::Perplexity
        | ProviderId::Nscale | ProviderId::PgVector | ProviderId::S3Vectors | ProviderId::Valkey | ProviderId::Mongodb | ProviderId::Helicone | ProviderId::Hyperbolic | ProviderId::Recraft | ProviderId::FalAi | ProviderId::Stability | ProviderId::Heroku | ProviderId::Aiml
        | ProviderId::Cometapi | ProviderId::Oci | ProviderId::AutoRouter | ProviderId::VercelAiGateway | ProviderId::Edenai | ProviderId::Dotprompt | ProviderId::Manus | ProviderId::Wandb | ProviderId::Ovhcloud | ProviderId::Scaleway | ProviderId::Lemonade | ProviderId::AmazonNova
        | ProviderId::A2aAgent | ProviderId::Langgraph | ProviderId::Langflow | ProviderId::Minimax | ProviderId::Synthetic | ProviderId::Apertis | ProviderId::Nanogpt | ProviderId::Poe | ProviderId::Chutes | ProviderId::Abliteration | ProviderId::Llamagate | ProviderId::Gmi
        | ProviderId::Sarvam | ProviderId::Aihubmix | ProviderId::Crusoe | ProviderId::Empiriolabs | ProviderId::Veniceai | ProviderId::Neosantara | ProviderId::Parasail | ProviderId::XiaomiMimo | ProviderId::Tensormesh | ProviderId::Libertai | ProviderId::Pinstripes | ProviderId::Cognition
        | ProviderId::Cortecs | ProviderId::ScxAi | ProviderId::Prism | ProviderId::Darkbloom | ProviderId::Meta | ProviderId::Sail | ProviderId::LitellmAgent | ProviderId::Cursor | ProviderId::BedrockMantle | ProviderId::Gdc => None,
    }
}

// the call every regional embedding arm starts from; each arm sets its own litellm_params and headers
pub(super) fn embedding_call(context: &EmbeddingContext<'_>) -> EmbeddingCall {
    EmbeddingCall {
        model: context.model.to_owned(),
        input: context.input.clone(),
        optional_params: context.optional_params.clone(),
        litellm_params: Object::new(),
        api_key: context.api_key.cloned(),
        api_base: context.api_base.map(str::to_owned),
        headers: context.extra_headers.cloned(),
        timeout: context.timeout,
        aembedding: context.aembedding,
    }
}

// the three DashScope brands read the passed key, litellm.api_key, their own env key and DASHSCOPE_API_KEY, and name the brand they were called as when none is set
fn family(
    brand: DashscopeFamily,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingCall, DispatchError> {
    let passed = first_secret([context.api_key, context.settings.api_key.as_ref()]);
    let key = dashscope_family::resolve_family_api_key(
        brand,
        passed.as_ref().map(ExposeSecret::expose_secret),
        brand_key(brand, context),
        context.settings.environment.dashscope_api_key.as_ref(),
    )
    .ok_or_else(|| DispatchError::MissingKey {
        message: dashscope_family::missing_family_key_message(brand),
    })?;
    let mut call = embedding_call(context);
    call.api_key = Some(key);
    call.headers = Some(context.extra_headers.cloned().unwrap_or_default());
    call.litellm_params = Object::new();
    Ok(call)
}

const fn brand_key<'request>(
    brand: DashscopeFamily,
    context: &EmbeddingContext<'request>,
) -> Option<&'request SecretString> {
    match brand {
        DashscopeFamily::Dashscope => None,
        DashscopeFamily::QwenCloud => context.settings.environment.qwencloud_key.as_ref(),
        DashscopeFamily::QwenAiPlatform => {
            context.settings.environment.qwen_ai_platform_key.as_ref()
        }
    }
}

// the gigachat arm, with get_llm_provider's GIGACHAT_API_KEY env already in the passed key: litellm.api_key, gigachat_key and GIGACHAT_CREDENTIALS follow; litellm_params holds only ssl_verify
fn gigachat(context: &EmbeddingContext<'_>) -> EmbeddingCall {
    let mut call = embedding_call(context);
    call.api_key = first_secret([
        context.api_key,
        context.settings.environment.gigachat_api_key.as_ref(),
        context.settings.api_key.as_ref(),
        context.settings.gigachat_key.as_ref(),
        context.settings.environment.gigachat_credentials.as_ref(),
    ]);
    call.headers = None;
    call.litellm_params = Object::from_iter([(
        String::from("ssl_verify"),
        context.ssl_verify.map_or(Json::Null, Json::Bool),
    )]);
    call
}
