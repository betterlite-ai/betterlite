use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::compatible_embedding::Mode;
use betterlite_openai::embedding::EmbeddingInput;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::compatible_embedding_call::SharedEmbedding;

use super::call::{EmbeddingCall, EmbeddingProvider, EmbeddingRequest};
use super::context::{EmbeddingContext, first_nonempty, first_secret, ollama_api_base};
use super::error::DispatchError;
use super::logging;

const XINFERENCE_API_BASE: &str = "XINFERENCE_API_BASE";
const XINFERENCE_API_KEY: &str = "XINFERENCE_API_KEY";
const XINFERENCE_DEFAULT_API_BASE: &str = "http://127.0.0.1:9997/v1";
const XINFERENCE_DEFAULT_KEY: &str = "stub-xinference-key";

// main.py's embedding arms: five providers resolve their own base and key, and llamafile/lm_studio go through B1440;
// the skip keeps the exhaustive no-op arm within clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch(
    provider: ProviderId,
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Option<Result<EmbeddingRequest, DispatchError>> {
    match provider {
        ProviderId::HostedVllm => Some(Ok(hosted_vllm(call, context))),
        ProviderId::Triton => Some(triton(call)),
        ProviderId::Oobabooga => Some(Ok(oobabooga(call))),
        ProviderId::Ollama => Some(ollama(call, context)),
        ProviderId::Xinference => Some(xinference(call, context)),
        ProviderId::Llamafile => Some(shared(EmbeddingProvider::Llamafile, call, context)),
        ProviderId::LmStudio => Some(shared(EmbeddingProvider::LmStudio, call, context)),
        ProviderId::Openai | ProviderId::Chatgpt | ProviderId::OpenaiLike | ProviderId::JinaAi | ProviderId::Xai | ProviderId::Zai | ProviderId::CustomOpenai | ProviderId::TextCompletionOpenai | ProviderId::Cohere | ProviderId::CohereChat | ProviderId::Clarifai | ProviderId::Anthropic
        | ProviderId::AnthropicText | ProviderId::Bytez | ProviderId::Replicate | ProviderId::Reducto | ProviderId::AwsTextract | ProviderId::Runwayml | ProviderId::AwsPolly | ProviderId::Transcribe | ProviderId::Huggingface | ProviderId::TogetherAi | ProviderId::Openrouter | ProviderId::Datarobot
        | ProviderId::VertexAi | ProviderId::VertexAiBeta | ProviderId::Gemini | ProviderId::Ai21 | ProviderId::Baseten | ProviderId::BlackForestLabs | ProviderId::Azure | ProviderId::AzureText | ProviderId::AzureAi | ProviderId::Sagemaker | ProviderId::SagemakerChat | ProviderId::SagemakerNova
        | ProviderId::Bedrock | ProviderId::Vllm | ProviderId::NlpCloud | ProviderId::Petals | ProviderId::OllamaChat | ProviderId::Deepinfra | ProviderId::Perplexity | ProviderId::Typesafe | ProviderId::StrandsDecider | ProviderId::Mistral | ProviderId::Milvus | ProviderId::Groq
        | ProviderId::A2a | ProviderId::Gigachat | ProviderId::NvidiaNim | ProviderId::NvidiaRiva | ProviderId::Soniox | ProviderId::Cerebras | ProviderId::Nadir | ProviderId::Ai21Chat | ProviderId::Volcengine | ProviderId::Codestral | ProviderId::TextCompletionCodestral | ProviderId::Dashscope
        | ProviderId::Qwencloud | ProviderId::QwenAiPlatform | ProviderId::Modelscope | ProviderId::Moonshot | ProviderId::Publicai | ProviderId::V0 | ProviderId::Morph | ProviderId::LambdaAi | ProviderId::Inception | ProviderId::TextCompletionInception | ProviderId::Deepseek | ProviderId::Sambanova
        | ProviderId::Maritalk | ProviderId::Voyage | ProviderId::Cloudflare | ProviderId::FireworksAi | ProviderId::Friendliai | ProviderId::FeatherlessAi | ProviderId::Watsonx | ProviderId::WatsonxText | ProviderId::Predibase | ProviderId::Databricks | ProviderId::Empower | ProviderId::Github
        | ProviderId::Ragflow | ProviderId::Compactifai | ProviderId::DockerModelRunner | ProviderId::Custom | ProviderId::LitellmProxy | ProviderId::Tencent | ProviderId::Galadriel | ProviderId::Nebius | ProviderId::Infinity | ProviderId::Deepgram | ProviderId::Elevenlabs | ProviderId::Novita
        | ProviderId::AiohttpOpenai | ProviderId::Langfuse | ProviderId::Humanloop | ProviderId::Topaz | ProviderId::SapGenerativeAiHub | ProviderId::Assemblyai | ProviderId::AzureSpeech | ProviderId::CharityEngine | ProviderId::GithubCopilot | ProviderId::Snowflake | ProviderId::GradientAi | ProviderId::Llama
        | ProviderId::Nscale | ProviderId::PgVector | ProviderId::S3Vectors | ProviderId::Valkey | ProviderId::Mongodb | ProviderId::Helicone | ProviderId::Hyperbolic | ProviderId::Recraft | ProviderId::FalAi | ProviderId::Stability | ProviderId::Heroku | ProviderId::Aiml
        | ProviderId::Cometapi | ProviderId::Oci | ProviderId::AutoRouter | ProviderId::VercelAiGateway | ProviderId::Edenai | ProviderId::Dotprompt | ProviderId::Manus | ProviderId::Wandb | ProviderId::Ovhcloud | ProviderId::Scaleway | ProviderId::Lemonade | ProviderId::AmazonNova
        | ProviderId::A2aAgent | ProviderId::Langgraph | ProviderId::Langflow | ProviderId::Minimax | ProviderId::Synthetic | ProviderId::Apertis | ProviderId::Nanogpt | ProviderId::Poe | ProviderId::Chutes | ProviderId::Abliteration | ProviderId::Llamagate | ProviderId::Gmi
        | ProviderId::Sarvam | ProviderId::Aihubmix | ProviderId::Crusoe | ProviderId::Empiriolabs | ProviderId::Veniceai | ProviderId::Neosantara | ProviderId::Parasail | ProviderId::XiaomiMimo | ProviderId::Tensormesh | ProviderId::Libertai | ProviderId::Pinstripes | ProviderId::Cognition
        | ProviderId::Cortecs | ProviderId::ScxAi | ProviderId::Prism | ProviderId::Darkbloom | ProviderId::Meta | ProviderId::Sail | ProviderId::LitellmAgent | ProviderId::Cursor | ProviderId::BedrockMantle | ProviderId::Gdc => None,
    }
}

// hosted_vllm: the passed base, then HOSTED_VLLM_API_BASE, then litellm.api_base; the key falls back to the env
fn hosted_vllm(call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> EmbeddingRequest {
    EmbeddingRequest::Compatible {
        provider: EmbeddingProvider::HostedVllm,
        api_base: first_nonempty([
            call.api_base.as_deref(),
            context.settings.hosted_vllm_api_base,
            context.settings.api_base,
        ]),
        api_key: first_secret([call.api_key.as_ref(), context.settings.hosted_vllm_api_key]),
        input: call.input.clone(),
        optional_params: call.optional_params.clone(),
        headers: call.headers.clone(),
    }
}

// triton: the passed base is required and the passed key goes through with no headers of its own
fn triton(call: &EmbeddingCall) -> Result<EmbeddingRequest, DispatchError> {
    let Some(api_base) = call.api_base.clone() else {
        return Err(DispatchError::MissingTritonApiBase);
    };
    Ok(EmbeddingRequest::Compatible {
        provider: EmbeddingProvider::Triton,
        api_base: Some(api_base),
        api_key: call.api_key.clone(),
        input: call.input.clone(),
        optional_params: call.optional_params.clone(),
        headers: None,
    })
}

// oobabooga: no base or key fallback at all, and the caller's headers never reach this arm
fn oobabooga(call: &EmbeddingCall) -> EmbeddingRequest {
    EmbeddingRequest::Compatible {
        provider: EmbeddingProvider::Oobabooga,
        api_base: call.api_base.clone(),
        api_key: call.api_key.clone(),
        input: call.input.clone(),
        optional_params: call.optional_params.clone(),
        headers: None,
    }
}

// ollama: the chat base chain, prompts made of strings only, and no key on the request
fn ollama(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingRequest, DispatchError> {
    // all() over an empty token list is true, so Python sends it as an empty prompt list
    let prompts = match &call.input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(tokens) if tokens.is_empty() => Vec::new(),
        EmbeddingInput::TokenLists(lists) if lists.is_empty() => Vec::new(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(DispatchError::OllamaEmbeddingInput);
        }
    };
    Ok(EmbeddingRequest::Ollama {
        api_base: ollama_api_base(context.settings, call.api_base.as_deref()),
        model: call.model.clone(),
        prompts,
        optional_params: call.optional_params.clone(),
    })
}

// xinference: the passed values, litellm's globals, then the environment, then the local stub base and key
fn xinference(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingRequest, DispatchError> {
    let api_key = first_secret([call.api_key.as_ref(), context.settings.api_key])
        .or_else(|| {
            environment_value(context.environment, XINFERENCE_API_KEY).map(SecretString::from)
        })
        .unwrap_or_else(|| SecretString::from(XINFERENCE_DEFAULT_KEY));
    let api_base = first_nonempty([call.api_base.as_deref(), context.settings.api_base])
        .or_else(|| environment_value(context.environment, XINFERENCE_API_BASE))
        .unwrap_or_else(|| String::from(XINFERENCE_DEFAULT_API_BASE));
    Ok(EmbeddingRequest::Compatible {
        provider: EmbeddingProvider::Xinference,
        api_base: Some(api_base),
        api_key: Some(api_key),
        input: call.input.clone(),
        optional_params: call.optional_params.clone(),
        headers: None,
    })
}

// llamafile and lm_studio hand their resolution and request to B1440's shared openai_like embedding call
fn shared(
    provider: EmbeddingProvider,
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingRequest, DispatchError> {
    let input: DecisionsJson = logging::convert(&call.input)?;
    let optional_params: IndexMap<String, DecisionsJson> = logging::convert(&call.optional_params)?;
    let embedding = SharedEmbedding {
        model: &call.model,
        input: &input,
        api_base: call.api_base.as_deref(),
        api_key: call.api_key.as_ref(),
        optional_params: &optional_params,
        headers: call.headers.as_ref(),
        custom_endpoint: false,
        timeout: call.timeout,
        mode: if call.aembedding {
            Mode::Async
        } else {
            Mode::Sync
        },
        globals: context.globals,
        settings: context.compatible,
    };
    Ok(EmbeddingRequest::Shared {
        provider,
        call: Box::new(embedding.handler_call()),
    })
}

fn environment_value(environment: &dyn EnvironmentLookup, name: &str) -> Option<String> {
    environment.lookup(name).filter(|value| !value.is_empty())
}
