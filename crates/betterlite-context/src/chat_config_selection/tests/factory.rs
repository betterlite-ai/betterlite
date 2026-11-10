use betterlite_models::identity::provider_id::ProviderId;

use crate::bedrock_config_selection::chat::{BedrockInvokeProvider, BedrockRoute};

use super::super::facts::BedrockFacts;
use super::super::table::{PROVIDER_CONFIG_ROWS, ProviderConfigTable};
use super::super::tags::{ChatConfigTag, ConfigConstruction};
use super::harness::{CHAT_CATALOG, CallSpec, TEXT_CATALOG, World, make_world};

const FACTORY_CASES: &[(ProviderId, &str, &str)] = &[
    (ProviderId::Openai, "gpt-4o", "OpenAIGPTConfig"),
    (ProviderId::Chatgpt, "test-model", "ChatGPTConfig"),
    (ProviderId::OpenaiLike, "test-model", "OpenAILikeChatConfig"),
    (ProviderId::Xai, "test-model", "XAIChatConfig"),
    (ProviderId::Zai, "test-model", "ZAIChatConfig"),
    (
        ProviderId::CustomOpenai,
        "test-model",
        "OpenAILikeChatConfig",
    ),
    (
        ProviderId::TextCompletionOpenai,
        "test-model",
        "OpenAITextCompletionConfig",
    ),
    (ProviderId::Cohere, "v1/command", "CohereChatConfig"),
    (ProviderId::CohereChat, "command", "CohereV2ChatConfig"),
    (ProviderId::Clarifai, "test-model", "ClarifaiConfig"),
    (ProviderId::Anthropic, "test-model", "AnthropicConfig"),
    (
        ProviderId::AnthropicText,
        "test-model",
        "AnthropicTextConfig",
    ),
    (ProviderId::Bytez, "test-model", "BytezChatConfig"),
    (ProviderId::Replicate, "test-model", "ReplicateConfig"),
    (
        ProviderId::Huggingface,
        "test-model",
        "HuggingFaceChatConfig",
    ),
    (ProviderId::TogetherAi, "test-model", "TogetherAIChatConfig"),
    (ProviderId::Openrouter, "test-model", "OpenrouterConfig"),
    (ProviderId::Datarobot, "test-model", "DataRobotConfig"),
    (ProviderId::VertexAi, "gemini-pro", "VertexGeminiConfig"),
    (ProviderId::VertexAiBeta, "test-model", "VertexGeminiConfig"),
    (
        ProviderId::Gemini,
        "test-model",
        "GoogleAIStudioGeminiConfig",
    ),
    (ProviderId::Ai21, "test-model", "AI21ChatConfig"),
    (ProviderId::Baseten, "test-model", "BasetenConfig"),
    (ProviderId::AzureText, "test-model", "AzureOpenAITextConfig"),
    (ProviderId::AzureAi, "claude-sonnet", "AzureAnthropicConfig"),
    (ProviderId::Sagemaker, "test-model", "SagemakerConfig"),
    (
        ProviderId::SagemakerChat,
        "test-model",
        "SagemakerChatConfig",
    ),
    (
        ProviderId::SagemakerNova,
        "test-model",
        "SagemakerNovaConfig",
    ),
    (
        ProviderId::Bedrock,
        "anthropic.claude-v2",
        "AmazonAnthropicConfig",
    ),
    (ProviderId::Vllm, "test-model", "VLLMConfig"),
    (ProviderId::NlpCloud, "test-model", "NLPCloudConfig"),
    (ProviderId::Petals, "test-model", "PetalsConfig"),
    (ProviderId::Oobabooga, "test-model", "OobaboogaConfig"),
    (ProviderId::Ollama, "test-model", "OllamaConfig"),
    (ProviderId::OllamaChat, "test-model", "OllamaChatConfig"),
    (ProviderId::Deepinfra, "test-model", "DeepInfraConfig"),
    (ProviderId::Perplexity, "test-model", "PerplexityChatConfig"),
    (ProviderId::Mistral, "test-model", "MistralConfig"),
    (ProviderId::Groq, "test-model", "GroqChatConfig"),
    (ProviderId::A2a, "test-model", "A2AConfig"),
    (ProviderId::Gigachat, "test-model", "GigaChatConfig"),
    (ProviderId::NvidiaNim, "test-model", "NvidiaNimConfig"),
    (ProviderId::Cerebras, "test-model", "CerebrasConfig"),
    (ProviderId::Nadir, "test-model", "NadirConfig"),
    (ProviderId::Ai21Chat, "test-model", "AI21ChatConfig"),
    (ProviderId::Volcengine, "test-model", "VolcEngineConfig"),
    (ProviderId::Codestral, "test-model", "MistralConfig"),
    (
        ProviderId::TextCompletionCodestral,
        "test-model",
        "CodestralTextCompletionConfig",
    ),
    (ProviderId::Dashscope, "test-model", "DashScopeChatConfig"),
    (ProviderId::Qwencloud, "test-model", "QwenCloudChatConfig"),
    (
        ProviderId::QwenAiPlatform,
        "test-model",
        "QwenAIPlatformChatConfig",
    ),
    (ProviderId::Modelscope, "test-model", "ModelScopeChatConfig"),
    (ProviderId::Moonshot, "test-model", "MoonshotChatConfig"),
    (ProviderId::V0, "test-model", "V0ChatConfig"),
    (ProviderId::Morph, "test-model", "MorphChatConfig"),
    (ProviderId::LambdaAi, "test-model", "LambdaAIChatConfig"),
    (ProviderId::Inception, "test-model", "InceptionChatConfig"),
    (
        ProviderId::TextCompletionInception,
        "test-model",
        "InceptionTextCompletionConfig",
    ),
    (ProviderId::Deepseek, "test-model", "DeepSeekChatConfig"),
    (ProviderId::Sambanova, "test-model", "SambanovaConfig"),
    (ProviderId::Maritalk, "test-model", "MaritalkConfig"),
    (ProviderId::Cloudflare, "test-model", "CloudflareChatConfig"),
    (ProviderId::FireworksAi, "test-model", "FireworksAIConfig"),
    (ProviderId::Friendliai, "test-model", "FriendliaiChatConfig"),
    (
        ProviderId::FeatherlessAi,
        "test-model",
        "FeatherlessAIConfig",
    ),
    (ProviderId::Watsonx, "test-model", "IBMWatsonXChatConfig"),
    (ProviderId::WatsonxText, "test-model", "IBMWatsonXAIConfig"),
    (ProviderId::Triton, "test-model", "TritonConfig"),
    (ProviderId::Predibase, "test-model", "PredibaseConfig"),
    (ProviderId::Databricks, "test-model", "DatabricksConfig"),
    (ProviderId::Empower, "test-model", "EmpowerChatConfig"),
    (ProviderId::Github, "test-model", "GithubChatConfig"),
    (ProviderId::Ragflow, "test-model", "RAGFlowConfig"),
    (
        ProviderId::Compactifai,
        "test-model",
        "CompactifAIChatConfig",
    ),
    (
        ProviderId::DockerModelRunner,
        "test-model",
        "DockerModelRunnerChatConfig",
    ),
    (ProviderId::Custom, "test-model", "OpenAILikeChatConfig"),
    (
        ProviderId::LitellmProxy,
        "test-model",
        "LiteLLMProxyChatConfig",
    ),
    (ProviderId::HostedVllm, "test-model", "HostedVLLMChatConfig"),
    (ProviderId::Tencent, "test-model", "TencentChatConfig"),
    (ProviderId::Llamafile, "test-model", "LlamafileChatConfig"),
    (ProviderId::LmStudio, "test-model", "LMStudioChatConfig"),
    (ProviderId::Galadriel, "test-model", "GaladrielChatConfig"),
    (ProviderId::Nebius, "test-model", "NebiusConfig"),
    (ProviderId::Novita, "test-model", "NovitaConfig"),
    (
        ProviderId::AiohttpOpenai,
        "test-model",
        "AiohttpOpenAIChatConfig",
    ),
    (
        ProviderId::SapGenerativeAiHub,
        "test-model",
        "GenAIHubOrchestrationConfig",
    ),
    (
        ProviderId::GithubCopilot,
        "test-model",
        "GithubCopilotConfig",
    ),
    (ProviderId::Snowflake, "test-model", "SnowflakeConfig"),
    (ProviderId::GradientAi, "test-model", "GradientAIConfig"),
    (ProviderId::Llama, "test-model", "LlamaAPIConfig"),
    (ProviderId::Nscale, "test-model", "NscaleConfig"),
    (ProviderId::Hyperbolic, "test-model", "HyperbolicChatConfig"),
    (ProviderId::FalAi, "test-model", "FalAIChatConfig"),
    (ProviderId::Heroku, "test-model", "HerokuChatConfig"),
    (ProviderId::Cometapi, "test-model", "CometAPIConfig"),
    (ProviderId::Oci, "test-model", "OCIChatConfig"),
    (
        ProviderId::VercelAiGateway,
        "test-model",
        "VercelAIGatewayConfig",
    ),
    (ProviderId::Edenai, "test-model", "EdenAIChatConfig"),
    (ProviderId::Wandb, "test-model", "WandbConfig"),
    (ProviderId::Ovhcloud, "test-model", "OVHCloudChatConfig"),
    (ProviderId::AmazonNova, "test-model", "AmazonNovaChatConfig"),
    (ProviderId::Langgraph, "test-model", "LangGraphConfig"),
    (ProviderId::Langflow, "test-model", "LangFlowConfig"),
    (ProviderId::Minimax, "test-model", "MinimaxChatConfig"),
    (ProviderId::Sail, "test-model", "SailChatConfig"),
    (
        ProviderId::BedrockMantle,
        "test-model",
        "BedrockMantleChatConfig",
    ),
    (ProviderId::Gdc, "test-model", "GDCGeminiConfig"),
];

const MODEL_FACTORIES: &[ProviderId] = &[
    ProviderId::AzureAi,
    ProviderId::VertexAi,
    ProviderId::Bedrock,
    ProviderId::Cohere,
    ProviderId::CohereChat,
];

// LiteLLM derives the Bedrock route, provider and base model from the model name before the row runs
fn case_spec(provider: ProviderId, model: &'static str) -> CallSpec<'static> {
    if provider == ProviderId::Bedrock {
        CallSpec {
            bedrock: BedrockFacts {
                route: BedrockRoute::Invoke,
                invoke_provider: Some(BedrockInvokeProvider::Anthropic),
                base_model: model,
            },
            ..CallSpec::plain(provider, model)
        }
    } else {
        CallSpec::plain(provider, model)
    }
}

pub(super) fn factory_table() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    factory_table_rows(&world);
    factory_table_arguments(&world);
    factory_table_layout();
    factory_table_lazy_build(&world);
}

fn factory_table_rows(world: &World) {
    for &(provider, model, class) in FACTORY_CASES {
        let selection = world.select(&case_spec(provider, model)).unwrap().unwrap();
        assert_eq!(selection.tag.class_name(), class, "{provider:?}");
    }
}

fn factory_table_arguments(world: &World) {
    let mut model_aware = Vec::new();
    for &(provider, model, _) in FACTORY_CASES {
        let selection = world.select(&case_spec(provider, model)).unwrap().unwrap();
        let arguments = if MODEL_FACTORIES.contains(&provider) {
            model_aware.push(provider);
            vec![model.to_owned()]
        } else {
            Vec::new()
        };
        assert_eq!(
            selection.construction,
            ConfigConstruction::New(arguments),
            "{provider:?}"
        );
    }
    model_aware.sort_unstable();
    let mut expected = MODEL_FACTORIES.to_vec();
    expected.sort_unstable();
    assert_eq!(model_aware, expected);
}

fn factory_table_layout() {
    let mut listed: Vec<ProviderId> = PROVIDER_CONFIG_ROWS
        .direct
        .iter()
        .map(|(provider, _)| *provider)
        .chain(
            PROVIDER_CONFIG_ROWS
                .model_aware
                .iter()
                .map(|(provider, _)| *provider),
        )
        .collect();
    listed.sort_unstable();
    let mut expected: Vec<ProviderId> = FACTORY_CASES
        .iter()
        .map(|(provider, _, _)| *provider)
        .collect();
    expected.sort_unstable();
    assert_eq!(listed, expected);
    let mut model_aware: Vec<ProviderId> = PROVIDER_CONFIG_ROWS
        .model_aware
        .iter()
        .map(|(provider, _)| *provider)
        .collect();
    model_aware.sort_unstable();
    let mut expected_factories = MODEL_FACTORIES.to_vec();
    expected_factories.sort_unstable();
    assert_eq!(model_aware, expected_factories);
}

fn factory_table_lazy_build(world: &World) {
    let mut table = ProviderConfigTable::default();
    assert!(table.rows.is_none());
    assert!(
        world
            .select_with(&CallSpec::plain(ProviderId::Openai, "o1"), &mut table)
            .unwrap()
            .is_some()
    );
    assert!(table.rows.is_none());
    assert!(
        world
            .select_with(&CallSpec::plain(ProviderId::Azure, "o1"), &mut table)
            .unwrap()
            .is_some()
    );
    assert!(table.rows.is_none());
    assert!(
        world
            .select_with(
                &CallSpec::plain(ProviderId::Anthropic, "test-model"),
                &mut table
            )
            .unwrap()
            .is_some()
    );
    assert!(table.rows.is_some());
    assert_eq!(
        world
            .select_with(&CallSpec::plain(ProviderId::Groq, "test-model"), &mut table)
            .unwrap()
            .map(|selection| selection.tag),
        Some(ChatConfigTag::GroqChat)
    );
    assert!(table.rows.is_some());
}
