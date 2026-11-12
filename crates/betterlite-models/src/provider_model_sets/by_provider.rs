use std::collections::{BTreeMap, BTreeSet};

use super::ProviderModelSet;

// LiteLLM's models_by_provider, one row per provider key, unions spelled out as the source builds them.
const PROVIDERS: [(&str, &[ProviderModelSet]); 101] = [
    (
        "openai",
        &[
            ProviderModelSet::OpenAiChatCompletion,
            ProviderModelSet::OpenAiTextCompletion,
        ],
    ),
    (
        "text-completion-openai",
        &[ProviderModelSet::OpenAiTextCompletion],
    ),
    (
        "cohere",
        &[ProviderModelSet::Cohere, ProviderModelSet::CohereChat],
    ),
    ("cohere_chat", &[ProviderModelSet::CohereChat]),
    ("anthropic", &[ProviderModelSet::Anthropic]),
    ("replicate", &[ProviderModelSet::Replicate]),
    ("huggingface", &[ProviderModelSet::Huggingface]),
    ("together_ai", &[ProviderModelSet::TogetherAi]),
    ("baseten", &[ProviderModelSet::Baseten]),
    ("openrouter", &[ProviderModelSet::Openrouter]),
    ("vercel_ai_gateway", &[ProviderModelSet::VercelAiGateway]),
    ("edenai", &[ProviderModelSet::Edenai]),
    ("datarobot", &[ProviderModelSet::Datarobot]),
    (
        "vertex_ai",
        &[
            ProviderModelSet::VertexZai,
            ProviderModelSet::VertexMoonshot,
            ProviderModelSet::VertexMinimax,
            ProviderModelSet::VertexDeepseek,
            ProviderModelSet::VertexLanguage,
            ProviderModelSet::VertexVision,
            ProviderModelSet::VertexAnthropic,
            ProviderModelSet::VertexChat,
            ProviderModelSet::VertexText,
        ],
    ),
    ("ai21", &[ProviderModelSet::Ai21]),
    (
        "bedrock",
        &[ProviderModelSet::Bedrock, ProviderModelSet::BedrockConverse],
    ),
    ("petals", &[ProviderModelSet::Petals]),
    ("ollama", &[ProviderModelSet::Ollama]),
    ("ollama_chat", &[ProviderModelSet::Ollama]),
    ("deepinfra", &[ProviderModelSet::Deepinfra]),
    ("perplexity", &[ProviderModelSet::Perplexity]),
    ("maritalk", &[ProviderModelSet::Maritalk]),
    ("watsonx", &[ProviderModelSet::Watsonx]),
    ("gemini", &[ProviderModelSet::Gemini]),
    (
        "fireworks_ai",
        &[
            ProviderModelSet::FireworksAi,
            ProviderModelSet::FireworksAiEmbedding,
        ],
    ),
    ("aleph_alpha", &[ProviderModelSet::AlephAlpha]),
    (
        "text-completion-codestral",
        &[ProviderModelSet::TextCompletionCodestral],
    ),
    (
        "text-completion-inception",
        &[ProviderModelSet::TextCompletionInception],
    ),
    ("xai", &[ProviderModelSet::Xai]),
    ("zai", &[ProviderModelSet::Zai]),
    ("fal_ai", &[ProviderModelSet::FalAi]),
    ("deepseek", &[ProviderModelSet::Deepseek]),
    ("tencent", &[ProviderModelSet::Tencent]),
    ("runwayml", &[ProviderModelSet::Runwayml]),
    ("mistral", &[ProviderModelSet::MistralChat]),
    ("azure_ai", &[ProviderModelSet::AzureAi]),
    ("voyage", &[ProviderModelSet::Voyage]),
    ("infinity", &[ProviderModelSet::Infinity]),
    ("databricks", &[ProviderModelSet::Databricks]),
    ("cloudflare", &[ProviderModelSet::Cloudflare]),
    ("codestral", &[ProviderModelSet::Codestral]),
    ("nlp_cloud", &[ProviderModelSet::NlpCloud]),
    ("friendliai", &[ProviderModelSet::Friendliai]),
    ("palm", &[ProviderModelSet::Palm]),
    ("groq", &[ProviderModelSet::Groq]),
    (
        "azure",
        &[ProviderModelSet::Azure, ProviderModelSet::AzureText],
    ),
    ("azure_anthropic", &[ProviderModelSet::AzureAnthropic]),
    ("azure_text", &[ProviderModelSet::AzureText]),
    ("anyscale", &[ProviderModelSet::Anyscale]),
    ("cerebras", &[ProviderModelSet::Cerebras]),
    ("nadir", &[ProviderModelSet::Nadir]),
    ("galadriel", &[ProviderModelSet::Galadriel]),
    ("nvidia_nim", &[ProviderModelSet::NvidiaNim]),
    ("nvidia_riva", &[ProviderModelSet::NvidiaRiva]),
    ("soniox", &[ProviderModelSet::Soniox]),
    (
        "sambanova",
        &[
            ProviderModelSet::Sambanova,
            ProviderModelSet::SambanovaEmbedding,
        ],
    ),
    ("novita", &[ProviderModelSet::Novita]),
    (
        "nebius",
        &[ProviderModelSet::Nebius, ProviderModelSet::NebiusEmbedding],
    ),
    ("aiml", &[ProviderModelSet::Aiml]),
    ("assemblyai", &[ProviderModelSet::Assemblyai]),
    ("jina_ai", &[ProviderModelSet::JinaAi]),
    ("snowflake", &[ProviderModelSet::Snowflake]),
    ("gradient_ai", &[ProviderModelSet::GradientAi]),
    ("meta_llama", &[ProviderModelSet::Llama]),
    ("nscale", &[ProviderModelSet::Nscale]),
    ("featherless_ai", &[ProviderModelSet::FeatherlessAi]),
    ("deepgram", &[ProviderModelSet::Deepgram]),
    ("elevenlabs", &[ProviderModelSet::Elevenlabs]),
    ("heroku", &[ProviderModelSet::Heroku]),
    ("dashscope", &[ProviderModelSet::Dashscope]),
    ("qwencloud", &[ProviderModelSet::Qwencloud]),
    ("qwen_ai_platform", &[ProviderModelSet::QwenAiPlatform]),
    ("modelscope", &[ProviderModelSet::Modelscope]),
    ("moonshot", &[ProviderModelSet::Moonshot]),
    ("publicai", &[ProviderModelSet::Publicai]),
    ("darkbloom", &[ProviderModelSet::Darkbloom]),
    ("v0", &[ProviderModelSet::V0]),
    ("morph", &[ProviderModelSet::Morph]),
    ("lambda_ai", &[ProviderModelSet::LambdaAi]),
    ("inception", &[ProviderModelSet::Inception]),
    ("hyperbolic", &[ProviderModelSet::Hyperbolic]),
    ("black_forest_labs", &[ProviderModelSet::BlackForestLabs]),
    ("recraft", &[ProviderModelSet::Recraft]),
    ("cometapi", &[ProviderModelSet::Cometapi]),
    ("oci", &[ProviderModelSet::Oci]),
    ("volcengine", &[ProviderModelSet::Volcengine]),
    ("wandb", &[ProviderModelSet::Wandb]),
    (
        "ovhcloud",
        &[
            ProviderModelSet::Ovhcloud,
            ProviderModelSet::OvhcloudEmbedding,
        ],
    ),
    ("lemonade", &[ProviderModelSet::Lemonade]),
    ("clarifai", &[ProviderModelSet::Clarifai]),
    ("amazon_nova", &[ProviderModelSet::AmazonNova]),
    ("stability", &[ProviderModelSet::Stability]),
    ("github_copilot", &[ProviderModelSet::GithubCopilot]),
    ("chatgpt", &[ProviderModelSet::Chatgpt]),
    ("minimax", &[ProviderModelSet::Minimax]),
    ("aws_polly", &[ProviderModelSet::AwsPolly]),
    ("transcribe", &[ProviderModelSet::Transcribe]),
    ("gigachat", &[ProviderModelSet::Gigachat]),
    ("llamagate", &[ProviderModelSet::Llamagate]),
    ("reducto", &[ProviderModelSet::Reducto]),
    ("bedrock_mantle", &[ProviderModelSet::BedrockMantle]),
];

pub(super) fn build(
    sets: &BTreeMap<ProviderModelSet, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut by_provider = BTreeMap::new();
    for (provider, members) in PROVIDERS {
        let mut names = BTreeSet::new();
        for set in members {
            if let Some(found) = sets.get(set) {
                names.extend(found.iter().cloned());
            }
        }
        by_provider.insert(provider.to_owned(), names);
    }
    by_provider
}
