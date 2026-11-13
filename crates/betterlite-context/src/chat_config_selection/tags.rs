use crate::bedrock_config_selection::chat::BedrockChatConfigTag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSelection<Tag> {
    pub tag: Tag,
    pub construction: ConfigConstruction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigConstruction {
    // the source hands back one process-wide config object instead of constructing a new one
    Shared,
    New(Vec<String>),
}

macro_rules! chat_config_tags {
    ($($variant:ident $(($binding:ident : $payload:ty))? => $value:expr),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum ChatConfigTag {
            $($variant $(($payload))?,)*
        }

        impl ChatConfigTag {
            #[must_use]
            pub const fn class_name(self) -> &'static str {
                match self {
                    $(Self::$variant $(($binding))? => $value,)*
                }
            }
        }
    };
}

chat_config_tags! {
    A2a => "A2AConfig",
    Ai21Chat => "AI21ChatConfig",
    AiohttpOpenAiChat => "AiohttpOpenAIChatConfig",
    AmazonNovaChat => "AmazonNovaChatConfig",
    Anthropic => "AnthropicConfig",
    AnthropicText => "AnthropicTextConfig",
    AzureAiStudio => "AzureAIStudioConfig",
    AzureAnthropic => "AzureAnthropicConfig",
    AzureModelRouter => "AzureModelRouterConfig",
    AzureOpenAi => "AzureOpenAIConfig",
    AzureOpenAiGpt5 => "AzureOpenAIGPT5Config",
    AzureOpenAiO1 => "AzureOpenAIO1Config",
    AzureOpenAiText => "AzureOpenAITextConfig",
    Baseten => "BasetenConfig",
    Bedrock(tag: BedrockChatConfigTag) => tag.class_name(),
    BedrockMantleChat => "BedrockMantleChatConfig",
    BytezChat => "BytezChatConfig",
    Cerebras => "CerebrasConfig",
    ChatGpt => "ChatGPTConfig",
    Clarifai => "ClarifaiConfig",
    CloudflareChat => "CloudflareChatConfig",
    CodestralTextCompletion => "CodestralTextCompletionConfig",
    CohereChat => "CohereChatConfig",
    CohereV2Chat => "CohereV2ChatConfig",
    CometApi => "CometAPIConfig",
    CompactifAiChat => "CompactifAIChatConfig",
    DashScopeChat => "DashScopeChatConfig",
    DataRobot => "DataRobotConfig",
    Databricks => "DatabricksConfig",
    DeepInfra => "DeepInfraConfig",
    DeepSeekChat => "DeepSeekChatConfig",
    DockerModelRunnerChat => "DockerModelRunnerChatConfig",
    EdenAiChat => "EdenAIChatConfig",
    EmpowerChat => "EmpowerChatConfig",
    FalAiChat => "FalAIChatConfig",
    FeatherlessAi => "FeatherlessAIConfig",
    FireworksAi => "FireworksAIConfig",
    FriendliaiChat => "FriendliaiChatConfig",
    GaladrielChat => "GaladrielChatConfig",
    GdcGemini => "GDCGeminiConfig",
    GenAiHubOrchestration => "GenAIHubOrchestrationConfig",
    GigaChat => "GigaChatConfig",
    GithubChat => "GithubChatConfig",
    GithubCopilot => "GithubCopilotConfig",
    GoogleAiStudioGemini => "GoogleAIStudioGeminiConfig",
    GradientAi => "GradientAIConfig",
    GroqChat => "GroqChatConfig",
    HerokuChat => "HerokuChatConfig",
    HostedVllmChat => "HostedVLLMChatConfig",
    HuggingFaceChat => "HuggingFaceChatConfig",
    HyperbolicChat => "HyperbolicChatConfig",
    IbmWatsonXAi => "IBMWatsonXAIConfig",
    IbmWatsonXChat => "IBMWatsonXChatConfig",
    InceptionChat => "InceptionChatConfig",
    InceptionTextCompletion => "InceptionTextCompletionConfig",
    JsonProvider => "JSONProviderConfig",
    LambdaAiChat => "LambdaAIChatConfig",
    LangFlow => "LangFlowConfig",
    LangGraph => "LangGraphConfig",
    LitellmProxyChat => "LiteLLMProxyChatConfig",
    LlamaApi => "LlamaAPIConfig",
    LlamafileChat => "LlamafileChatConfig",
    LmStudioChat => "LMStudioChatConfig",
    Maritalk => "MaritalkConfig",
    MinimaxChat => "MinimaxChatConfig",
    Mistral => "MistralConfig",
    ModelScopeChat => "ModelScopeChatConfig",
    MoonshotChat => "MoonshotChatConfig",
    MorphChat => "MorphChatConfig",
    Nadir => "NadirConfig",
    Nebius => "NebiusConfig",
    NlpCloud => "NLPCloudConfig",
    Novita => "NovitaConfig",
    Nscale => "NscaleConfig",
    NvidiaNim => "NvidiaNimConfig",
    OciChat => "OCIChatConfig",
    Ollama => "OllamaConfig",
    OllamaChat => "OllamaChatConfig",
    Oobabooga => "OobaboogaConfig",
    OpenAiGpt => "OpenAIGPTConfig",
    OpenAiGpt5 => "OpenAIGPT5Config",
    OpenAiLikeChat => "OpenAILikeChatConfig",
    OpenAiOSeries => "OpenAIOSeriesConfig",
    OpenAiTextCompletion => "OpenAITextCompletionConfig",
    OpenAiUnknownModel => "OpenAIUnknownModelConfig",
    Openrouter => "OpenrouterConfig",
    OvhCloudChat => "OVHCloudChatConfig",
    PerplexityChat => "PerplexityChatConfig",
    Petals => "PetalsConfig",
    Predibase => "PredibaseConfig",
    QwenAiPlatformChat => "QwenAIPlatformChatConfig",
    QwenCloudChat => "QwenCloudChatConfig",
    RagFlow => "RAGFlowConfig",
    Replicate => "ReplicateConfig",
    Sagemaker => "SagemakerConfig",
    SagemakerChat => "SagemakerChatConfig",
    SagemakerNova => "SagemakerNovaConfig",
    SailChat => "SailChatConfig",
    Sambanova => "SambanovaConfig",
    Snowflake => "SnowflakeConfig",
    TencentChat => "TencentChatConfig",
    TogetherAiChat => "TogetherAIChatConfig",
    Triton => "TritonConfig",
    V0Chat => "V0ChatConfig",
    VercelAiGateway => "VercelAIGatewayConfig",
    VertexAiAi21 => "VertexAIAi21Config",
    VertexAiAnthropic => "VertexAIAnthropicConfig",
    VertexAiGptOss => "VertexAIGPTOSSTransformation",
    VertexAiLlama3 => "VertexAILlama3Config",
    VertexAiMistral => "VertexAIMistralConfig",
    VertexGemini => "VertexGeminiConfig",
    Vllm => "VLLMConfig",
    VolcEngine => "VolcEngineConfig",
    Wandb => "WandbConfig",
    XaiChat => "XAIChatConfig",
    ZaiChat => "ZAIChatConfig",
}
