use crate::metadata::identity::Mode;

use super::ProviderModelSet;

const VERTEX_PREFIX: &str = "vertex_ai/";
const FIREWORKS_DEFAULT: &str = "fireworks-ai-default";
const FIREWORKS_PRICING: &str = "-to-";
const VERTEX_STRIPPED: [ProviderModelSet; 11] = [
    ProviderModelSet::VertexAnthropic,
    ProviderModelSet::VertexLlama3,
    ProviderModelSet::VertexDeepseek,
    ProviderModelSet::VertexMistral,
    ProviderModelSet::VertexAiAi21,
    ProviderModelSet::VertexAiImage,
    ProviderModelSet::VertexAiVideo,
    ProviderModelSet::VertexOpenai,
    ProviderModelSet::VertexMinimax,
    ProviderModelSet::VertexMoonshot,
    ProviderModelSet::VertexZai,
];

// One row per litellm_provider the population chain names, sorted for the lookup below.
const PROVIDER_SETS: [(&str, ProviderModelSet); 114] = [
    ("aiml", ProviderModelSet::Aiml),
    ("aleph_alpha", ProviderModelSet::AlephAlpha),
    ("amazon_nova", ProviderModelSet::AmazonNova),
    ("anthropic", ProviderModelSet::Anthropic),
    ("anyscale", ProviderModelSet::Anyscale),
    ("assemblyai", ProviderModelSet::Assemblyai),
    ("aws_polly", ProviderModelSet::AwsPolly),
    ("azure", ProviderModelSet::Azure),
    ("azure_ai", ProviderModelSet::AzureAi),
    ("azure_anthropic", ProviderModelSet::AzureAnthropic),
    ("azure_text", ProviderModelSet::AzureText),
    ("bedrock_converse", ProviderModelSet::BedrockConverse),
    ("bedrock_mantle", ProviderModelSet::BedrockMantle),
    ("black_forest_labs", ProviderModelSet::BlackForestLabs),
    ("cerebras", ProviderModelSet::Cerebras),
    ("chatgpt", ProviderModelSet::Chatgpt),
    ("cloudflare", ProviderModelSet::Cloudflare),
    ("codestral", ProviderModelSet::Codestral),
    ("cohere", ProviderModelSet::Cohere),
    ("cohere_chat", ProviderModelSet::CohereChat),
    ("cometapi", ProviderModelSet::Cometapi),
    ("darkbloom", ProviderModelSet::Darkbloom),
    ("dashscope", ProviderModelSet::Dashscope),
    ("databricks", ProviderModelSet::Databricks),
    ("datarobot", ProviderModelSet::Datarobot),
    ("deepgram", ProviderModelSet::Deepgram),
    ("deepinfra", ProviderModelSet::Deepinfra),
    ("deepseek", ProviderModelSet::Deepseek),
    ("docker_model_runner", ProviderModelSet::DockerModelRunner),
    ("edenai", ProviderModelSet::Edenai),
    ("elevenlabs", ProviderModelSet::Elevenlabs),
    ("empower", ProviderModelSet::Empower),
    ("fal_ai", ProviderModelSet::FalAi),
    ("featherless_ai", ProviderModelSet::FeatherlessAi),
    ("fireworks_ai", ProviderModelSet::FireworksAi),
    (
        "fireworks_ai-embedding-models",
        ProviderModelSet::FireworksAiEmbedding,
    ),
    ("friendliai", ProviderModelSet::Friendliai),
    ("galadriel", ProviderModelSet::Galadriel),
    ("gemini", ProviderModelSet::Gemini),
    ("gigachat", ProviderModelSet::Gigachat),
    ("github_copilot", ProviderModelSet::GithubCopilot),
    ("gradient_ai", ProviderModelSet::GradientAi),
    ("groq", ProviderModelSet::Groq),
    ("heroku", ProviderModelSet::Heroku),
    ("hyperbolic", ProviderModelSet::Hyperbolic),
    ("inception", ProviderModelSet::Inception),
    ("infinity", ProviderModelSet::Infinity),
    ("jina_ai", ProviderModelSet::JinaAi),
    ("lambda_ai", ProviderModelSet::LambdaAi),
    ("lemonade", ProviderModelSet::Lemonade),
    ("llamagate", ProviderModelSet::Llamagate),
    ("meta_llama", ProviderModelSet::Llama),
    ("minimax", ProviderModelSet::Minimax),
    ("mistral", ProviderModelSet::MistralChat),
    ("modelscope", ProviderModelSet::Modelscope),
    ("moonshot", ProviderModelSet::Moonshot),
    ("morph", ProviderModelSet::Morph),
    ("nadir", ProviderModelSet::Nadir),
    ("nebius-chat-models", ProviderModelSet::Nebius),
    ("nebius-embedding-models", ProviderModelSet::NebiusEmbedding),
    ("nlp_cloud", ProviderModelSet::NlpCloud),
    ("novita", ProviderModelSet::Novita),
    ("nscale", ProviderModelSet::Nscale),
    ("nvidia_nim", ProviderModelSet::NvidiaNim),
    ("nvidia_riva", ProviderModelSet::NvidiaRiva),
    ("oci", ProviderModelSet::Oci),
    ("openai", ProviderModelSet::OpenAiChatCompletion),
    ("openrouter", ProviderModelSet::Openrouter),
    ("ovhcloud", ProviderModelSet::Ovhcloud),
    (
        "ovhcloud-embedding-models",
        ProviderModelSet::OvhcloudEmbedding,
    ),
    ("palm", ProviderModelSet::Palm),
    ("perplexity", ProviderModelSet::Perplexity),
    ("publicai", ProviderModelSet::Publicai),
    ("qwen_ai_platform", ProviderModelSet::QwenAiPlatform),
    ("qwencloud", ProviderModelSet::Qwencloud),
    ("recraft", ProviderModelSet::Recraft),
    ("reducto", ProviderModelSet::Reducto),
    ("runwayml", ProviderModelSet::Runwayml),
    ("sambanova", ProviderModelSet::Sambanova),
    (
        "sambanova-embedding-models",
        ProviderModelSet::SambanovaEmbedding,
    ),
    ("snowflake", ProviderModelSet::Snowflake),
    ("soniox", ProviderModelSet::Soniox),
    ("stability", ProviderModelSet::Stability),
    ("tencent", ProviderModelSet::Tencent),
    (
        "text-completion-codestral",
        ProviderModelSet::TextCompletionCodestral,
    ),
    (
        "text-completion-inception",
        ProviderModelSet::TextCompletionInception,
    ),
    (
        "text-completion-openai",
        ProviderModelSet::OpenAiTextCompletion,
    ),
    ("transcribe", ProviderModelSet::Transcribe),
    ("v0", ProviderModelSet::V0),
    ("vercel_ai_gateway", ProviderModelSet::VercelAiGateway),
    ("vertex_ai-ai21_models", ProviderModelSet::VertexAiAi21),
    (
        "vertex_ai-anthropic_models",
        ProviderModelSet::VertexAnthropic,
    ),
    ("vertex_ai-chat-models", ProviderModelSet::VertexChat),
    (
        "vertex_ai-code-chat-models",
        ProviderModelSet::VertexCodeChat,
    ),
    (
        "vertex_ai-code-text-models",
        ProviderModelSet::VertexCodeText,
    ),
    (
        "vertex_ai-deepseek_models",
        ProviderModelSet::VertexDeepseek,
    ),
    (
        "vertex_ai-embedding-models",
        ProviderModelSet::VertexEmbedding,
    ),
    ("vertex_ai-image-models", ProviderModelSet::VertexAiImage),
    (
        "vertex_ai-language-models",
        ProviderModelSet::VertexLanguage,
    ),
    ("vertex_ai-llama_models", ProviderModelSet::VertexLlama3),
    ("vertex_ai-minimax_models", ProviderModelSet::VertexMinimax),
    ("vertex_ai-mistral_models", ProviderModelSet::VertexMistral),
    (
        "vertex_ai-moonshot_models",
        ProviderModelSet::VertexMoonshot,
    ),
    ("vertex_ai-openai_models", ProviderModelSet::VertexOpenai),
    ("vertex_ai-text-models", ProviderModelSet::VertexText),
    ("vertex_ai-video-models", ProviderModelSet::VertexAiVideo),
    ("vertex_ai-vision-models", ProviderModelSet::VertexVision),
    ("vertex_ai-zai_models", ProviderModelSet::VertexZai),
    ("volcengine", ProviderModelSet::Volcengine),
    ("voyage", ProviderModelSet::Voyage),
    ("wandb", ProviderModelSet::Wandb),
    ("watsonx", ProviderModelSet::Watsonx),
    ("xai", ProviderModelSet::Xai),
    ("zai", ProviderModelSet::Zai),
];

pub(super) fn set_for(provider: &str, mode: Option<Mode>) -> Option<ProviderModelSet> {
    match provider {
        "ai21" if mode == Some(Mode::Chat) => return Some(ProviderModelSet::Ai21Chat),
        "ai21" => return Some(ProviderModelSet::Ai21),
        "bedrock" if mode == Some(Mode::Guardrail) => return None,
        "bedrock" => return Some(ProviderModelSet::Bedrock),
        _ => {}
    }
    PROVIDER_SETS
        .binary_search_by_key(&provider, |(name, _)| *name)
        .ok()
        .and_then(|index| PROVIDER_SETS.get(index))
        .map(|(_, set)| *set)
}

// The skips and the vertex prefix are the chain's branches that transform a key rather than add it.
pub(super) fn name_for(set: ProviderModelSet, key: &str) -> Option<String> {
    if set == ProviderModelSet::OpenAiChatCompletion
        && key.starts_with("ft:")
        && key.matches(':').count() <= 1
    {
        return None;
    }
    if set == ProviderModelSet::Bedrock && is_bedrock_pricing_key(key) {
        return None;
    }
    if set == ProviderModelSet::FireworksAi
        && (key.contains(FIREWORKS_PRICING) || key.contains(FIREWORKS_DEFAULT))
    {
        return None;
    }
    if set == ProviderModelSet::FireworksAiEmbedding && key.contains(FIREWORKS_PRICING) {
        return None;
    }
    if VERTEX_STRIPPED.contains(&set) {
        return Some(key.replace(VERTEX_PREFIX, ""));
    }
    Some(key.to_owned())
}

fn is_bedrock_pricing_key(key: &str) -> bool {
    if key.contains("month-commitment") {
        return true;
    }
    let Some(rest) = key.strip_prefix("bedrock/") else {
        return false;
    };
    let Some((region, model)) = rest.split_once('/') else {
        return false;
    };
    !region.is_empty()
        && !model.is_empty()
        && region.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}
