use strum::VariantArray as _;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
)]
pub enum ProviderId {
    #[serde(rename = "openai")]
    #[strum(serialize = "openai")]
    Openai,
    #[serde(rename = "chatgpt")]
    #[strum(serialize = "chatgpt")]
    Chatgpt,
    #[serde(rename = "openai_like")]
    #[strum(serialize = "openai_like")]
    OpenaiLike,
    #[serde(rename = "jina_ai")]
    #[strum(serialize = "jina_ai")]
    JinaAi,
    #[serde(rename = "xai")]
    #[strum(serialize = "xai")]
    Xai,
    #[serde(rename = "zai")]
    #[strum(serialize = "zai")]
    Zai,
    #[serde(rename = "custom_openai")]
    #[strum(serialize = "custom_openai")]
    CustomOpenai,
    // Hand-mapped: the wire slug uses hyphens where the variant name uses words.
    #[serde(rename = "text-completion-openai")]
    #[strum(serialize = "text-completion-openai")]
    TextCompletionOpenai,
    #[serde(rename = "cohere")]
    #[strum(serialize = "cohere")]
    Cohere,
    #[serde(rename = "cohere_chat")]
    #[strum(serialize = "cohere_chat")]
    CohereChat,
    #[serde(rename = "clarifai")]
    #[strum(serialize = "clarifai")]
    Clarifai,
    #[serde(rename = "anthropic")]
    #[strum(serialize = "anthropic")]
    Anthropic,
    #[serde(rename = "anthropic_text")]
    #[strum(serialize = "anthropic_text")]
    AnthropicText,
    #[serde(rename = "bytez")]
    #[strum(serialize = "bytez")]
    Bytez,
    #[serde(rename = "replicate")]
    #[strum(serialize = "replicate")]
    Replicate,
    #[serde(rename = "reducto")]
    #[strum(serialize = "reducto")]
    Reducto,
    #[serde(rename = "aws_textract")]
    #[strum(serialize = "aws_textract")]
    AwsTextract,
    #[serde(rename = "runwayml")]
    #[strum(serialize = "runwayml")]
    Runwayml,
    #[serde(rename = "aws_polly")]
    #[strum(serialize = "aws_polly")]
    AwsPolly,
    #[serde(rename = "transcribe")]
    #[strum(serialize = "transcribe")]
    Transcribe,
    #[serde(rename = "huggingface")]
    #[strum(serialize = "huggingface")]
    Huggingface,
    #[serde(rename = "together_ai")]
    #[strum(serialize = "together_ai")]
    TogetherAi,
    #[serde(rename = "openrouter")]
    #[strum(serialize = "openrouter")]
    Openrouter,
    #[serde(rename = "datarobot")]
    #[strum(serialize = "datarobot")]
    Datarobot,
    #[serde(rename = "vertex_ai")]
    #[strum(serialize = "vertex_ai")]
    VertexAi,
    #[serde(rename = "vertex_ai_beta")]
    #[strum(serialize = "vertex_ai_beta")]
    VertexAiBeta,
    #[serde(rename = "gemini")]
    #[strum(serialize = "gemini")]
    Gemini,
    #[serde(rename = "ai21")]
    #[strum(serialize = "ai21")]
    Ai21,
    #[serde(rename = "baseten")]
    #[strum(serialize = "baseten")]
    Baseten,
    #[serde(rename = "black_forest_labs")]
    #[strum(serialize = "black_forest_labs")]
    BlackForestLabs,
    #[serde(rename = "azure")]
    #[strum(serialize = "azure")]
    Azure,
    #[serde(rename = "azure_text")]
    #[strum(serialize = "azure_text")]
    AzureText,
    #[serde(rename = "azure_ai")]
    #[strum(serialize = "azure_ai")]
    AzureAi,
    #[serde(rename = "sagemaker")]
    #[strum(serialize = "sagemaker")]
    Sagemaker,
    #[serde(rename = "sagemaker_chat")]
    #[strum(serialize = "sagemaker_chat")]
    SagemakerChat,
    #[serde(rename = "sagemaker_nova")]
    #[strum(serialize = "sagemaker_nova")]
    SagemakerNova,
    #[serde(rename = "bedrock")]
    #[strum(serialize = "bedrock")]
    Bedrock,
    #[serde(rename = "vllm")]
    #[strum(serialize = "vllm")]
    Vllm,
    #[serde(rename = "nlp_cloud")]
    #[strum(serialize = "nlp_cloud")]
    NlpCloud,
    #[serde(rename = "petals")]
    #[strum(serialize = "petals")]
    Petals,
    #[serde(rename = "oobabooga")]
    #[strum(serialize = "oobabooga")]
    Oobabooga,
    #[serde(rename = "ollama")]
    #[strum(serialize = "ollama")]
    Ollama,
    #[serde(rename = "ollama_chat")]
    #[strum(serialize = "ollama_chat")]
    OllamaChat,
    #[serde(rename = "deepinfra")]
    #[strum(serialize = "deepinfra")]
    Deepinfra,
    #[serde(rename = "perplexity")]
    #[strum(serialize = "perplexity")]
    Perplexity,
    #[serde(rename = "typesafe")]
    #[strum(serialize = "typesafe")]
    Typesafe,
    #[serde(rename = "strands_decider")]
    #[strum(serialize = "strands_decider")]
    StrandsDecider,
    #[serde(rename = "mistral")]
    #[strum(serialize = "mistral")]
    Mistral,
    #[serde(rename = "milvus")]
    #[strum(serialize = "milvus")]
    Milvus,
    #[serde(rename = "groq")]
    #[strum(serialize = "groq")]
    Groq,
    #[serde(rename = "a2a")]
    #[strum(serialize = "a2a")]
    A2a,
    #[serde(rename = "gigachat")]
    #[strum(serialize = "gigachat")]
    Gigachat,
    #[serde(rename = "nvidia_nim")]
    #[strum(serialize = "nvidia_nim")]
    NvidiaNim,
    #[serde(rename = "nvidia_riva")]
    #[strum(serialize = "nvidia_riva")]
    NvidiaRiva,
    #[serde(rename = "soniox")]
    #[strum(serialize = "soniox")]
    Soniox,
    #[serde(rename = "cerebras")]
    #[strum(serialize = "cerebras")]
    Cerebras,
    #[serde(rename = "nadir")]
    #[strum(serialize = "nadir")]
    Nadir,
    #[serde(rename = "ai21_chat")]
    #[strum(serialize = "ai21_chat")]
    Ai21Chat,
    #[serde(rename = "volcengine")]
    #[strum(serialize = "volcengine")]
    Volcengine,
    #[serde(rename = "codestral")]
    #[strum(serialize = "codestral")]
    Codestral,
    #[serde(rename = "text-completion-codestral")]
    #[strum(serialize = "text-completion-codestral")]
    TextCompletionCodestral,
    #[serde(rename = "dashscope")]
    #[strum(serialize = "dashscope")]
    Dashscope,
    #[serde(rename = "qwencloud")]
    #[strum(serialize = "qwencloud")]
    Qwencloud,
    #[serde(rename = "qwen_ai_platform")]
    #[strum(serialize = "qwen_ai_platform")]
    QwenAiPlatform,
    #[serde(rename = "modelscope")]
    #[strum(serialize = "modelscope")]
    Modelscope,
    #[serde(rename = "moonshot")]
    #[strum(serialize = "moonshot")]
    Moonshot,
    #[serde(rename = "publicai")]
    #[strum(serialize = "publicai")]
    Publicai,
    #[serde(rename = "v0")]
    #[strum(serialize = "v0")]
    V0,
    #[serde(rename = "morph")]
    #[strum(serialize = "morph")]
    Morph,
    #[serde(rename = "lambda_ai")]
    #[strum(serialize = "lambda_ai")]
    LambdaAi,
    #[serde(rename = "inception")]
    #[strum(serialize = "inception")]
    Inception,
    #[serde(rename = "text-completion-inception")]
    #[strum(serialize = "text-completion-inception")]
    TextCompletionInception,
    #[serde(rename = "deepseek")]
    #[strum(serialize = "deepseek")]
    Deepseek,
    #[serde(rename = "sambanova")]
    #[strum(serialize = "sambanova")]
    Sambanova,
    #[serde(rename = "maritalk")]
    #[strum(serialize = "maritalk")]
    Maritalk,
    #[serde(rename = "voyage")]
    #[strum(serialize = "voyage")]
    Voyage,
    #[serde(rename = "cloudflare")]
    #[strum(serialize = "cloudflare")]
    Cloudflare,
    #[serde(rename = "xinference")]
    #[strum(serialize = "xinference")]
    Xinference,
    #[serde(rename = "fireworks_ai")]
    #[strum(serialize = "fireworks_ai")]
    FireworksAi,
    #[serde(rename = "friendliai")]
    #[strum(serialize = "friendliai")]
    Friendliai,
    #[serde(rename = "featherless_ai")]
    #[strum(serialize = "featherless_ai")]
    FeatherlessAi,
    #[serde(rename = "watsonx")]
    #[strum(serialize = "watsonx")]
    Watsonx,
    #[serde(rename = "watsonx_text")]
    #[strum(serialize = "watsonx_text")]
    WatsonxText,
    #[serde(rename = "triton")]
    #[strum(serialize = "triton")]
    Triton,
    #[serde(rename = "predibase")]
    #[strum(serialize = "predibase")]
    Predibase,
    #[serde(rename = "databricks")]
    #[strum(serialize = "databricks")]
    Databricks,
    #[serde(rename = "empower")]
    #[strum(serialize = "empower")]
    Empower,
    #[serde(rename = "github")]
    #[strum(serialize = "github")]
    Github,
    #[serde(rename = "ragflow")]
    #[strum(serialize = "ragflow")]
    Ragflow,
    #[serde(rename = "compactifai")]
    #[strum(serialize = "compactifai")]
    Compactifai,
    #[serde(rename = "docker_model_runner")]
    #[strum(serialize = "docker_model_runner")]
    DockerModelRunner,
    #[serde(rename = "custom")]
    #[strum(serialize = "custom")]
    Custom,
    #[serde(rename = "litellm_proxy")]
    #[strum(serialize = "litellm_proxy")]
    LitellmProxy,
    #[serde(rename = "hosted_vllm")]
    #[strum(serialize = "hosted_vllm")]
    HostedVllm,
    #[serde(rename = "tencent")]
    #[strum(serialize = "tencent")]
    Tencent,
    #[serde(rename = "llamafile")]
    #[strum(serialize = "llamafile")]
    Llamafile,
    #[serde(rename = "lm_studio")]
    #[strum(serialize = "lm_studio")]
    LmStudio,
    #[serde(rename = "galadriel")]
    #[strum(serialize = "galadriel")]
    Galadriel,
    #[serde(rename = "nebius")]
    #[strum(serialize = "nebius")]
    Nebius,
    #[serde(rename = "infinity")]
    #[strum(serialize = "infinity")]
    Infinity,
    #[serde(rename = "deepgram")]
    #[strum(serialize = "deepgram")]
    Deepgram,
    #[serde(rename = "elevenlabs")]
    #[strum(serialize = "elevenlabs")]
    Elevenlabs,
    #[serde(rename = "novita")]
    #[strum(serialize = "novita")]
    Novita,
    #[serde(rename = "aiohttp_openai")]
    #[strum(serialize = "aiohttp_openai")]
    AiohttpOpenai,
    #[serde(rename = "langfuse")]
    #[strum(serialize = "langfuse")]
    Langfuse,
    #[serde(rename = "humanloop")]
    #[strum(serialize = "humanloop")]
    Humanloop,
    #[serde(rename = "topaz")]
    #[strum(serialize = "topaz")]
    Topaz,
    #[serde(rename = "sap")]
    #[strum(serialize = "sap")]
    SapGenerativeAiHub,
    #[serde(rename = "assemblyai")]
    #[strum(serialize = "assemblyai")]
    Assemblyai,
    #[serde(rename = "azure_speech")]
    #[strum(serialize = "azure_speech")]
    AzureSpeech,
    #[serde(rename = "charity_engine")]
    #[strum(serialize = "charity_engine")]
    CharityEngine,
    #[serde(rename = "github_copilot")]
    #[strum(serialize = "github_copilot")]
    GithubCopilot,
    #[serde(rename = "snowflake")]
    #[strum(serialize = "snowflake")]
    Snowflake,
    #[serde(rename = "gradient_ai")]
    #[strum(serialize = "gradient_ai")]
    GradientAi,
    // Hand-mapped: the variant is named after the model family, not the wire slug.
    #[serde(rename = "meta_llama")]
    #[strum(serialize = "meta_llama")]
    Llama,
    #[serde(rename = "nscale")]
    #[strum(serialize = "nscale")]
    Nscale,
    #[serde(rename = "pg_vector")]
    #[strum(serialize = "pg_vector")]
    PgVector,
    #[serde(rename = "s3_vectors")]
    #[strum(serialize = "s3_vectors")]
    S3Vectors,
    #[serde(rename = "valkey")]
    #[strum(serialize = "valkey")]
    Valkey,
    #[serde(rename = "mongodb")]
    #[strum(serialize = "mongodb")]
    Mongodb,
    #[serde(rename = "helicone")]
    #[strum(serialize = "helicone")]
    Helicone,
    #[serde(rename = "hyperbolic")]
    #[strum(serialize = "hyperbolic")]
    Hyperbolic,
    #[serde(rename = "recraft")]
    #[strum(serialize = "recraft")]
    Recraft,
    #[serde(rename = "fal_ai")]
    #[strum(serialize = "fal_ai")]
    FalAi,
    #[serde(rename = "stability")]
    #[strum(serialize = "stability")]
    Stability,
    #[serde(rename = "heroku")]
    #[strum(serialize = "heroku")]
    Heroku,
    #[serde(rename = "aiml")]
    #[strum(serialize = "aiml")]
    Aiml,
    #[serde(rename = "cometapi")]
    #[strum(serialize = "cometapi")]
    Cometapi,
    #[serde(rename = "oci")]
    #[strum(serialize = "oci")]
    Oci,
    #[serde(rename = "auto_router")]
    #[strum(serialize = "auto_router")]
    AutoRouter,
    #[serde(rename = "vercel_ai_gateway")]
    #[strum(serialize = "vercel_ai_gateway")]
    VercelAiGateway,
    #[serde(rename = "edenai")]
    #[strum(serialize = "edenai")]
    Edenai,
    #[serde(rename = "dotprompt")]
    #[strum(serialize = "dotprompt")]
    Dotprompt,
    #[serde(rename = "manus")]
    #[strum(serialize = "manus")]
    Manus,
    #[serde(rename = "wandb")]
    #[strum(serialize = "wandb")]
    Wandb,
    #[serde(rename = "ovhcloud")]
    #[strum(serialize = "ovhcloud")]
    Ovhcloud,
    #[serde(rename = "scaleway")]
    #[strum(serialize = "scaleway")]
    Scaleway,
    #[serde(rename = "lemonade")]
    #[strum(serialize = "lemonade")]
    Lemonade,
    #[serde(rename = "amazon_nova")]
    #[strum(serialize = "amazon_nova")]
    AmazonNova,
    #[serde(rename = "a2a_agent")]
    #[strum(serialize = "a2a_agent")]
    A2aAgent,
    #[serde(rename = "langgraph")]
    #[strum(serialize = "langgraph")]
    Langgraph,
    #[serde(rename = "langflow")]
    #[strum(serialize = "langflow")]
    Langflow,
    #[serde(rename = "minimax")]
    #[strum(serialize = "minimax")]
    Minimax,
    #[serde(rename = "synthetic")]
    #[strum(serialize = "synthetic")]
    Synthetic,
    #[serde(rename = "apertis")]
    #[strum(serialize = "apertis")]
    Apertis,
    // Hand-mapped: the wire slug hyphenates where the variant name does not.
    #[serde(rename = "nano-gpt")]
    #[strum(serialize = "nano-gpt")]
    Nanogpt,
    #[serde(rename = "poe")]
    #[strum(serialize = "poe")]
    Poe,
    #[serde(rename = "chutes")]
    #[strum(serialize = "chutes")]
    Chutes,
    // JSONProviderRegistry-only slugs (providers.json, B1424): no enum member in litellm.types.utils.LlmProviders.
    #[serde(rename = "abliteration")]
    #[strum(serialize = "abliteration")]
    Abliteration,
    #[serde(rename = "llamagate")]
    #[strum(serialize = "llamagate")]
    Llamagate,
    #[serde(rename = "gmi")]
    #[strum(serialize = "gmi")]
    Gmi,
    #[serde(rename = "sarvam")]
    #[strum(serialize = "sarvam")]
    Sarvam,
    #[serde(rename = "aihubmix")]
    #[strum(serialize = "aihubmix")]
    Aihubmix,
    #[serde(rename = "crusoe")]
    #[strum(serialize = "crusoe")]
    Crusoe,
    #[serde(rename = "empiriolabs")]
    #[strum(serialize = "empiriolabs")]
    Empiriolabs,
    #[serde(rename = "veniceai")]
    #[strum(serialize = "veniceai")]
    Veniceai,
    #[serde(rename = "neosantara")]
    #[strum(serialize = "neosantara")]
    Neosantara,
    #[serde(rename = "parasail")]
    #[strum(serialize = "parasail")]
    Parasail,
    #[serde(rename = "xiaomi_mimo")]
    #[strum(serialize = "xiaomi_mimo")]
    XiaomiMimo,
    #[serde(rename = "tensormesh")]
    #[strum(serialize = "tensormesh")]
    Tensormesh,
    #[serde(rename = "libertai")]
    #[strum(serialize = "libertai")]
    Libertai,
    #[serde(rename = "pinstripes")]
    #[strum(serialize = "pinstripes")]
    Pinstripes,
    #[serde(rename = "cognition")]
    #[strum(serialize = "cognition")]
    Cognition,
    #[serde(rename = "cortecs")]
    #[strum(serialize = "cortecs")]
    Cortecs,
    #[serde(rename = "scx-ai")]
    #[strum(serialize = "scx-ai")]
    ScxAi,
    #[serde(rename = "prism")]
    #[strum(serialize = "prism")]
    Prism,
    #[serde(rename = "darkbloom")]
    #[strum(serialize = "darkbloom")]
    Darkbloom,
    #[serde(rename = "meta")]
    #[strum(serialize = "meta")]
    Meta,
    #[serde(rename = "sail")]
    #[strum(serialize = "sail")]
    Sail,
    #[serde(rename = "litellm_agent")]
    #[strum(serialize = "litellm_agent")]
    LitellmAgent,
    #[serde(rename = "cursor")]
    #[strum(serialize = "cursor")]
    Cursor,
    #[serde(rename = "bedrock_mantle")]
    #[strum(serialize = "bedrock_mantle")]
    BedrockMantle,
    #[serde(rename = "gdc")]
    #[strum(serialize = "gdc")]
    Gdc,
}

impl ProviderId {
    pub const ALL: &'static [Self] = Self::VARIANTS;
}

mod tests;
