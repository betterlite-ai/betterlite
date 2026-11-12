use std::collections::{BTreeMap, BTreeSet};

use strum::VariantArray as _;

mod add;
mod by_provider;
mod provider;
mod tests;

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, strum::Display, strum::VariantArray,
)]
pub enum ProviderModelSet {
    #[strum(serialize = "open_ai_chat_completion_models")]
    OpenAiChatCompletion,
    #[strum(serialize = "open_ai_text_completion_models")]
    OpenAiTextCompletion,
    #[strum(serialize = "cohere_models")]
    Cohere,
    #[strum(serialize = "cohere_chat_models")]
    CohereChat,
    #[strum(serialize = "mistral_chat_models")]
    MistralChat,
    #[strum(serialize = "text_completion_codestral_models")]
    TextCompletionCodestral,
    #[strum(serialize = "text_completion_inception_models")]
    TextCompletionInception,
    #[strum(serialize = "anthropic_models")]
    Anthropic,
    #[strum(serialize = "openrouter_models")]
    Openrouter,
    #[strum(serialize = "datarobot_models")]
    Datarobot,
    #[strum(serialize = "vertex_language_models")]
    VertexLanguage,
    #[strum(serialize = "vertex_vision_models")]
    VertexVision,
    #[strum(serialize = "vertex_chat_models")]
    VertexChat,
    #[strum(serialize = "vertex_code_chat_models")]
    VertexCodeChat,
    #[strum(serialize = "vertex_ai_image_models")]
    VertexAiImage,
    #[strum(serialize = "vertex_ai_video_models")]
    VertexAiVideo,
    #[strum(serialize = "vertex_text_models")]
    VertexText,
    #[strum(serialize = "vertex_code_text_models")]
    VertexCodeText,
    #[strum(serialize = "vertex_embedding_models")]
    VertexEmbedding,
    #[strum(serialize = "vertex_anthropic_models")]
    VertexAnthropic,
    #[strum(serialize = "vertex_llama3_models")]
    VertexLlama3,
    #[strum(serialize = "vertex_deepseek_models")]
    VertexDeepseek,
    #[strum(serialize = "vertex_ai_ai21_models")]
    VertexAiAi21,
    #[strum(serialize = "vertex_mistral_models")]
    VertexMistral,
    #[strum(serialize = "vertex_openai_models")]
    VertexOpenai,
    #[strum(serialize = "vertex_minimax_models")]
    VertexMinimax,
    #[strum(serialize = "vertex_moonshot_models")]
    VertexMoonshot,
    #[strum(serialize = "vertex_zai_models")]
    VertexZai,
    #[strum(serialize = "ai21_models")]
    Ai21,
    #[strum(serialize = "ai21_chat_models")]
    Ai21Chat,
    #[strum(serialize = "nlp_cloud_models")]
    NlpCloud,
    #[strum(serialize = "aleph_alpha_models")]
    AlephAlpha,
    #[strum(serialize = "bedrock_models")]
    Bedrock,
    #[strum(serialize = "bedrock_converse_models")]
    BedrockConverse,
    #[strum(serialize = "fal_ai_models")]
    FalAi,
    #[strum(serialize = "fireworks_ai_models")]
    FireworksAi,
    #[strum(serialize = "fireworks_ai_embedding_models")]
    FireworksAiEmbedding,
    #[strum(serialize = "deepinfra_models")]
    Deepinfra,
    #[strum(serialize = "perplexity_models")]
    Perplexity,
    #[strum(serialize = "watsonx_models")]
    Watsonx,
    #[strum(serialize = "gemini_models")]
    Gemini,
    #[strum(serialize = "xai_models")]
    Xai,
    #[strum(serialize = "zai_models")]
    Zai,
    #[strum(serialize = "deepseek_models")]
    Deepseek,
    #[strum(serialize = "tencent_models")]
    Tencent,
    #[strum(serialize = "runwayml_models")]
    Runwayml,
    #[strum(serialize = "azure_ai_models")]
    AzureAi,
    #[strum(serialize = "jina_ai_models")]
    JinaAi,
    #[strum(serialize = "voyage_models")]
    Voyage,
    #[strum(serialize = "infinity_models")]
    Infinity,
    #[strum(serialize = "heroku_models")]
    Heroku,
    #[strum(serialize = "databricks_models")]
    Databricks,
    #[strum(serialize = "cloudflare_models")]
    Cloudflare,
    #[strum(serialize = "codestral_models")]
    Codestral,
    #[strum(serialize = "friendliai_models")]
    Friendliai,
    #[strum(serialize = "featherless_ai_models")]
    FeatherlessAi,
    #[strum(serialize = "palm_models")]
    Palm,
    #[strum(serialize = "groq_models")]
    Groq,
    #[strum(serialize = "azure_models")]
    Azure,
    #[strum(serialize = "azure_anthropic_models")]
    AzureAnthropic,
    #[strum(serialize = "azure_text_models")]
    AzureText,
    #[strum(serialize = "anyscale_models")]
    Anyscale,
    #[strum(serialize = "cerebras_models")]
    Cerebras,
    #[strum(serialize = "nadir_models")]
    Nadir,
    #[strum(serialize = "galadriel_models")]
    Galadriel,
    #[strum(serialize = "nvidia_nim_models")]
    NvidiaNim,
    #[strum(serialize = "nvidia_riva_models")]
    NvidiaRiva,
    #[strum(serialize = "soniox_models")]
    Soniox,
    #[strum(serialize = "sambanova_models")]
    Sambanova,
    #[strum(serialize = "sambanova_embedding_models")]
    SambanovaEmbedding,
    #[strum(serialize = "novita_models")]
    Novita,
    #[strum(serialize = "assemblyai_models")]
    Assemblyai,
    #[strum(serialize = "snowflake_models")]
    Snowflake,
    #[strum(serialize = "gradient_ai_models")]
    GradientAi,
    #[strum(serialize = "llama_models")]
    Llama,
    #[strum(serialize = "nscale_models")]
    Nscale,
    #[strum(serialize = "nebius_models")]
    Nebius,
    #[strum(serialize = "nebius_embedding_models")]
    NebiusEmbedding,
    #[strum(serialize = "aiml_models")]
    Aiml,
    #[strum(serialize = "deepgram_models")]
    Deepgram,
    #[strum(serialize = "elevenlabs_models")]
    Elevenlabs,
    #[strum(serialize = "dashscope_models")]
    Dashscope,
    #[strum(serialize = "qwencloud_models")]
    Qwencloud,
    #[strum(serialize = "qwen_ai_platform_models")]
    QwenAiPlatform,
    #[strum(serialize = "moonshot_models")]
    Moonshot,
    #[strum(serialize = "publicai_models")]
    Publicai,
    #[strum(serialize = "darkbloom_models")]
    Darkbloom,
    #[strum(serialize = "v0_models")]
    V0,
    #[strum(serialize = "morph_models")]
    Morph,
    #[strum(serialize = "lambda_ai_models")]
    LambdaAi,
    #[strum(serialize = "inception_models")]
    Inception,
    #[strum(serialize = "hyperbolic_models")]
    Hyperbolic,
    #[strum(serialize = "black_forest_labs_models")]
    BlackForestLabs,
    #[strum(serialize = "recraft_models")]
    Recraft,
    #[strum(serialize = "cometapi_models")]
    Cometapi,
    #[strum(serialize = "oci_models")]
    Oci,
    #[strum(serialize = "vercel_ai_gateway_models")]
    VercelAiGateway,
    #[strum(serialize = "edenai_models")]
    Edenai,
    #[strum(serialize = "volcengine_models")]
    Volcengine,
    #[strum(serialize = "wandb_models")]
    Wandb,
    #[strum(serialize = "ovhcloud_models")]
    Ovhcloud,
    #[strum(serialize = "ovhcloud_embedding_models")]
    OvhcloudEmbedding,
    #[strum(serialize = "lemonade_models")]
    Lemonade,
    #[strum(serialize = "docker_model_runner_models")]
    DockerModelRunner,
    #[strum(serialize = "amazon_nova_models")]
    AmazonNova,
    #[strum(serialize = "stability_models")]
    Stability,
    #[strum(serialize = "github_copilot_models")]
    GithubCopilot,
    #[strum(serialize = "chatgpt_models")]
    Chatgpt,
    #[strum(serialize = "minimax_models")]
    Minimax,
    #[strum(serialize = "aws_polly_models")]
    AwsPolly,
    #[strum(serialize = "transcribe_models")]
    Transcribe,
    #[strum(serialize = "gigachat_models")]
    Gigachat,
    #[strum(serialize = "llamagate_models")]
    Llamagate,
    #[strum(serialize = "reducto_models")]
    Reducto,
    #[strum(serialize = "bedrock_mantle_models")]
    BedrockMantle,
    #[strum(serialize = "replicate_models")]
    Replicate,
    #[strum(serialize = "clarifai_models")]
    Clarifai,
    #[strum(serialize = "huggingface_models")]
    Huggingface,
    #[strum(serialize = "modelscope_models")]
    Modelscope,
    #[strum(serialize = "empower_models")]
    Empower,
    #[strum(serialize = "together_ai_models")]
    TogetherAi,
    #[strum(serialize = "baseten_models")]
    Baseten,
    #[strum(serialize = "petals_models")]
    Petals,
    #[strum(serialize = "ollama_models")]
    Ollama,
    #[strum(serialize = "maritalk_models")]
    Maritalk,
}

#[derive(Debug)]
pub struct ProviderModelSets {
    pub sets: BTreeMap<ProviderModelSet, BTreeSet<String>>,
    pub models_by_provider: BTreeMap<String, BTreeSet<String>>,
}

impl Default for ProviderModelSets {
    fn default() -> Self {
        let sets = ProviderModelSet::VARIANTS
            .iter()
            .map(|set| (*set, BTreeSet::new()))
            .collect();
        Self {
            models_by_provider: by_provider::build(&sets),
            sets,
        }
    }
}
