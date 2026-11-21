use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::vercel_ai_gateway::DEFAULT_API_BASE as VERCEL_DEFAULT_API_BASE;

use super::call::EmbeddingCall;
use super::chat::OPENROUTER_DEFAULT_API_BASE;
use super::context::{DispatchSettings, first_nonempty, first_secret, referer_headers};

const OPENROUTER_REFERER: &str = "HTTP-Referer";
const OPENROUTER_TITLE: &str = "X-Title";

// main.py's embedding arms; the skip keeps the exhaustive no-op arm within clippy's line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch(
    provider: ProviderId,
    call: &EmbeddingCall,
    settings: DispatchSettings<'_>,
) -> Option<EmbeddingCall> {
    match provider {
        ProviderId::Openrouter => Some(openrouter(call, settings)),
        ProviderId::VercelAiGateway => Some(vercel(call, settings)),
        ProviderId::FireworksAi | ProviderId::Openai | ProviderId::Chatgpt | ProviderId::OpenaiLike | ProviderId::JinaAi | ProviderId::Xai | ProviderId::Zai | ProviderId::CustomOpenai | ProviderId::TextCompletionOpenai | ProviderId::Cohere | ProviderId::CohereChat | ProviderId::Clarifai | ProviderId::Anthropic
        | ProviderId::AnthropicText | ProviderId::Bytez | ProviderId::Replicate | ProviderId::Reducto | ProviderId::AwsTextract | ProviderId::Runwayml | ProviderId::AwsPolly | ProviderId::Transcribe | ProviderId::Huggingface | ProviderId::TogetherAi | ProviderId::Datarobot
        | ProviderId::VertexAi | ProviderId::VertexAiBeta | ProviderId::Gemini | ProviderId::Ai21 | ProviderId::Baseten | ProviderId::BlackForestLabs | ProviderId::Azure | ProviderId::AzureText | ProviderId::AzureAi | ProviderId::Sagemaker | ProviderId::SagemakerChat | ProviderId::SagemakerNova
        | ProviderId::Bedrock | ProviderId::Vllm | ProviderId::NlpCloud | ProviderId::Petals | ProviderId::Ollama | ProviderId::OllamaChat | ProviderId::Oobabooga | ProviderId::Triton | ProviderId::HostedVllm | ProviderId::Llamafile | ProviderId::LmStudio | ProviderId::Xinference | ProviderId::Deepinfra | ProviderId::Typesafe | ProviderId::StrandsDecider | ProviderId::Mistral | ProviderId::Milvus | ProviderId::Groq
        | ProviderId::A2a | ProviderId::Gigachat | ProviderId::NvidiaNim | ProviderId::NvidiaRiva | ProviderId::Soniox | ProviderId::Cerebras | ProviderId::Nadir | ProviderId::Ai21Chat | ProviderId::Volcengine | ProviderId::Codestral | ProviderId::TextCompletionCodestral | ProviderId::Dashscope
        | ProviderId::Qwencloud | ProviderId::QwenAiPlatform | ProviderId::Modelscope | ProviderId::Moonshot | ProviderId::Publicai | ProviderId::V0 | ProviderId::Morph | ProviderId::LambdaAi | ProviderId::Inception | ProviderId::TextCompletionInception | ProviderId::Deepseek | ProviderId::Sambanova
        | ProviderId::Maritalk | ProviderId::Voyage | ProviderId::Cloudflare | ProviderId::Friendliai | ProviderId::FeatherlessAi | ProviderId::Watsonx | ProviderId::WatsonxText | ProviderId::Predibase | ProviderId::Databricks | ProviderId::Empower | ProviderId::Github
        | ProviderId::Ragflow | ProviderId::Compactifai | ProviderId::DockerModelRunner | ProviderId::Custom | ProviderId::LitellmProxy | ProviderId::Tencent | ProviderId::Galadriel | ProviderId::Nebius | ProviderId::Infinity | ProviderId::Deepgram | ProviderId::Elevenlabs | ProviderId::Novita
        | ProviderId::AiohttpOpenai | ProviderId::Langfuse | ProviderId::Humanloop | ProviderId::Topaz | ProviderId::SapGenerativeAiHub | ProviderId::Assemblyai | ProviderId::AzureSpeech | ProviderId::CharityEngine | ProviderId::GithubCopilot | ProviderId::Snowflake | ProviderId::GradientAi | ProviderId::Llama
        | ProviderId::Nscale | ProviderId::PgVector | ProviderId::S3Vectors | ProviderId::Valkey | ProviderId::Mongodb | ProviderId::Helicone | ProviderId::Hyperbolic | ProviderId::Recraft | ProviderId::FalAi | ProviderId::Stability | ProviderId::Heroku | ProviderId::Aiml
        | ProviderId::Cometapi | ProviderId::Oci | ProviderId::AutoRouter | ProviderId::Edenai | ProviderId::Dotprompt | ProviderId::Manus | ProviderId::Wandb | ProviderId::Ovhcloud | ProviderId::Scaleway | ProviderId::Lemonade | ProviderId::AmazonNova
        | ProviderId::A2aAgent | ProviderId::Langgraph | ProviderId::Langflow | ProviderId::Minimax | ProviderId::Synthetic | ProviderId::Apertis | ProviderId::Nanogpt | ProviderId::Poe | ProviderId::Chutes | ProviderId::Abliteration | ProviderId::Llamagate | ProviderId::Gmi
        | ProviderId::Sarvam | ProviderId::Aihubmix | ProviderId::Crusoe | ProviderId::Empiriolabs | ProviderId::Veniceai | ProviderId::Neosantara | ProviderId::Parasail | ProviderId::XiaomiMimo | ProviderId::Tensormesh | ProviderId::Libertai | ProviderId::Pinstripes | ProviderId::Cognition
        | ProviderId::Cortecs | ProviderId::ScxAi | ProviderId::Prism | ProviderId::Darkbloom | ProviderId::Meta | ProviderId::Sail | ProviderId::LitellmAgent | ProviderId::Cursor | ProviderId::BedrockMantle | ProviderId::Gdc | ProviderId::Perplexity => None,
    }
}

// the openrouter arm uses the chat base, key and referer rules
fn openrouter(call: &EmbeddingCall, settings: DispatchSettings<'_>) -> EmbeddingCall {
    let mut resolved = call.clone();
    resolved.api_base = Some(
        first_nonempty([
            call.api_base.as_deref(),
            settings.api_base,
            settings.openrouter_api_base,
        ])
        .unwrap_or_else(|| String::from(OPENROUTER_DEFAULT_API_BASE)),
    );
    resolved.api_key = first_secret([
        call.api_key.as_ref(),
        settings.api_key,
        settings.openrouter_key,
        settings.openrouter_api_key,
        settings.or_api_key,
    ]);
    resolved.headers = Some(referer_headers(
        call.headers.as_ref(),
        settings.headers,
        OPENROUTER_REFERER,
        OPENROUTER_TITLE,
        settings.or_site_url,
        settings.or_app_name,
    ));
    resolved
}

// the vercel arm adds VERCEL_OIDC_TOKEN after VERCEL_AI_GATEWAY_API_KEY, and sends the request headers alone
fn vercel(call: &EmbeddingCall, settings: DispatchSettings<'_>) -> EmbeddingCall {
    let mut resolved = call.clone();
    resolved.api_base = Some(
        first_nonempty([
            call.api_base.as_deref(),
            settings.api_base,
            settings.vercel_ai_gateway_api_base,
        ])
        .unwrap_or_else(|| String::from(VERCEL_DEFAULT_API_BASE)),
    );
    resolved.api_key = first_secret([
        call.api_key.as_ref(),
        settings.api_key,
        settings.vercel_ai_gateway_api_key,
        settings.vercel_oidc_token,
    ]);
    resolved.headers = call.headers.clone();
    resolved
}
