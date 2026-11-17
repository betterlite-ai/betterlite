use std::collections::BTreeMap;

use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::compatible_embedding::Mode;
use betterlite_openai::embedding::EmbeddingInput;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::compatible_embedding_call::SharedEmbedding;

use super::call::{EmbeddingCall, EmbeddingProvider, EmbeddingRequest, OllamaRequest};
use super::context::{EmbeddingContext, first_nonempty, first_secret, ollama_api_base};
use super::error::DispatchError;
use super::logging;

const XINFERENCE_API_BASE: &str = "XINFERENCE_API_BASE";
const XINFERENCE_API_KEY: &str = "XINFERENCE_API_KEY";
const XINFERENCE_DEFAULT_API_BASE: &str = "http://127.0.0.1:9997/v1";
const XINFERENCE_DEFAULT_KEY: &str = "stub-xinference-key";

type Request = Result<EmbeddingRequest, DispatchError>;

// main.py's embedding arms: five resolve base and key, llamafile/lm_studio go through B1440; the skip keeps the exhaustive no-op arm within clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch(
    provider: ProviderId,
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Option<Request> {
    match provider {
        ProviderId::HostedVllm => Some(Ok(hosted_vllm(call, context))),
        ProviderId::Triton => Some(triton(call)),
        ProviderId::Oobabooga => Some(Ok(oobabooga(call))),
        ProviderId::Ollama => Some(ollama(call, context)),
        ProviderId::Xinference => Some(Ok(xinference(call, context))),
        ProviderId::Llamafile => Some(EmbeddingProvider::Llamafile.shared(call, context)),
        ProviderId::LmStudio => Some(EmbeddingProvider::LmStudio.shared(call, context)),
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

// the compatible request fields every arm starts from, before it overrides its own resolution
struct CompatibleDraft {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
}

impl CompatibleDraft {
    fn from_call(call: &EmbeddingCall) -> Self {
        Self {
            api_base: call.api_base.clone(),
            api_key: call.api_key.clone(),
            headers: call.headers.clone(),
        }
    }

    fn request(self, provider: EmbeddingProvider, call: &EmbeddingCall) -> EmbeddingRequest {
        EmbeddingRequest::Compatible {
            provider,
            api_base: self.api_base,
            api_key: self.api_key,
            input: call.input.clone(),
            optional_params: call.optional_params.clone(),
            headers: self.headers,
        }
    }

    fn hosted_vllm(mut self, call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> Self {
        self.api_base = first_nonempty([
            call.api_base.as_deref(),
            context.settings.hosted_vllm_api_base,
            context.settings.api_base,
        ]);
        self.api_key = first_secret([call.api_key.as_ref(), context.settings.hosted_vllm_api_key]);
        self
    }

    fn triton(mut self) -> Result<Self, DispatchError> {
        if self.api_base.is_none() {
            return Err(DispatchError::MissingTritonApiBase);
        }
        self.headers = None;
        Ok(self)
    }

    fn oobabooga(mut self) -> Self {
        self.headers = None;
        self
    }

    fn xinference(mut self, call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> Self {
        let (api_base, api_key) = xinference_values(call, context);
        self.api_base = api_base;
        self.api_key = api_key;
        self.headers = None;
        self
    }
}

// main.py resolves each arm's base, key and headers before it hands the values to the provider config
fn hosted_vllm(call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> EmbeddingRequest {
    CompatibleDraft::from_call(call)
        .hosted_vllm(call, context)
        .request(EmbeddingProvider::HostedVllm, call)
}

fn triton(call: &EmbeddingCall) -> Request {
    Ok(CompatibleDraft::from_call(call)
        .triton()?
        .request(EmbeddingProvider::Triton, call))
}

fn oobabooga(call: &EmbeddingCall) -> EmbeddingRequest {
    CompatibleDraft::from_call(call)
        .oobabooga()
        .request(EmbeddingProvider::Oobabooga, call)
}

fn xinference(call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> EmbeddingRequest {
    CompatibleDraft::from_call(call)
        .xinference(call, context)
        .request(EmbeddingProvider::Xinference, call)
}

fn xinference_values(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> (Option<String>, Option<SecretString>) {
    let api_key = first_secret([call.api_key.as_ref(), context.settings.api_key])
        .or_else(|| {
            let found = context
                .environment
                .lookup(XINFERENCE_API_KEY)
                .filter(|value| !value.is_empty());
            found.map(SecretString::from)
        })
        .unwrap_or_else(|| SecretString::from(XINFERENCE_DEFAULT_KEY));
    let api_base = first_nonempty([call.api_base.as_deref(), context.settings.api_base])
        .or_else(|| {
            context
                .environment
                .lookup(XINFERENCE_API_BASE)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| String::from(XINFERENCE_DEFAULT_API_BASE));
    (Some(api_base), Some(api_key))
}

// ollama's arm: the chat base chain, prompts made of strings only, and no key on the request
fn ollama(call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> Request {
    let api_base = ollama_api_base(context.settings, call.api_base.as_deref());
    Ok(EmbeddingRequest::Ollama(OllamaRequest {
        api_base,
        model: call.model.clone(),
        prompts: prompts(&call.input)?,
        optional_params: call.optional_params.clone(),
    }))
}

// all() over an empty token list is true, so Python sends it as an empty prompt list
fn prompts(input: &EmbeddingInput) -> Result<Vec<String>, DispatchError> {
    match input {
        EmbeddingInput::Text(text) => Ok(vec![text.clone()]),
        EmbeddingInput::Texts(texts) => Ok(texts.clone()),
        EmbeddingInput::Tokens(tokens) if tokens.is_empty() => Ok(Vec::new()),
        EmbeddingInput::TokenLists(lists) if lists.is_empty() => Ok(Vec::new()),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            Err(DispatchError::OllamaEmbeddingInput)
        }
    }
}

// the json forms B1440's shared call reads, kept alive while its handler call is prepared
struct SharedInputs {
    input: DecisionsJson,
    optional_params: IndexMap<String, DecisionsJson>,
}

impl SharedInputs {
    fn convert(call: &EmbeddingCall) -> Result<Self, DispatchError> {
        Ok(Self {
            input: logging::convert(&call.input)?,
            optional_params: logging::convert(&call.optional_params)?,
        })
    }
}

impl EmbeddingProvider {
    // llamafile and lm_studio hand their resolution and request to B1440's shared openai_like embedding call
    fn shared(self, call: &EmbeddingCall, context: &EmbeddingContext<'_>) -> Request {
        let inputs = SharedInputs::convert(call)?;
        let view = shared_view(call, context, &inputs.input, &inputs.optional_params);
        Ok(EmbeddingRequest::Shared {
            provider: self,
            call: Box::new(view.handler_call()),
        })
    }
}

fn shared_view<'source>(
    call: &'source EmbeddingCall,
    context: &'source EmbeddingContext<'source>,
    input: &'source DecisionsJson,
    optional_params: &'source IndexMap<String, DecisionsJson>,
) -> SharedEmbedding<'source> {
    SharedEmbedding {
        model: &call.model,
        input,
        api_base: call.api_base.as_deref(),
        api_key: call.api_key.as_ref(),
        optional_params,
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
    }
}
