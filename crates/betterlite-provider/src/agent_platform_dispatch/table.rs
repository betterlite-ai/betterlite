use betterlite_context::chat_config_selection::tags::ChatConfigTag;

use super::a2a::complete_a2a;
use super::context::DispatchContext;
use super::error::DispatchError;
use super::handler::ChatCompletionHandler;
use super::langflow::complete_langflow;
use super::langgraph::complete_langgraph;

// LiteLLM's dispatch arms for the three agent platforms; the skip keeps the 113-variant no-op arm short enough for clippy's 40-line limit
#[rustfmt::skip]
#[must_use]
pub fn dispatch<R>(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &dyn ChatCompletionHandler<Reply = R>,
) -> Option<Result<R, DispatchError>> {
    match tag {
        ChatConfigTag::A2a => Some(complete_a2a(context, handler)),
        ChatConfigTag::LangGraph => Some(complete_langgraph(context, handler)),
        ChatConfigTag::LangFlow => Some(complete_langflow(context, handler)),
        ChatConfigTag::Ai21Chat | ChatConfigTag::AiohttpOpenAiChat | ChatConfigTag::AmazonNovaChat | ChatConfigTag::Anthropic | ChatConfigTag::AnthropicText
        | ChatConfigTag::AzureAiStudio | ChatConfigTag::AzureAnthropic | ChatConfigTag::AzureModelRouter | ChatConfigTag::AzureOpenAi | ChatConfigTag::AzureOpenAiGpt5
        | ChatConfigTag::AzureOpenAiO1 | ChatConfigTag::AzureOpenAiText | ChatConfigTag::Baseten | ChatConfigTag::Bedrock(_) | ChatConfigTag::BedrockMantleChat
        | ChatConfigTag::BytezChat | ChatConfigTag::Cerebras | ChatConfigTag::ChatGpt | ChatConfigTag::Clarifai | ChatConfigTag::CloudflareChat
        | ChatConfigTag::CodestralTextCompletion | ChatConfigTag::CohereChat | ChatConfigTag::CohereV2Chat | ChatConfigTag::CometApi | ChatConfigTag::CompactifAiChat
        | ChatConfigTag::DashScopeChat | ChatConfigTag::DataRobot | ChatConfigTag::Databricks | ChatConfigTag::DeepInfra | ChatConfigTag::DeepSeekChat
        | ChatConfigTag::DockerModelRunnerChat | ChatConfigTag::EdenAiChat | ChatConfigTag::EmpowerChat | ChatConfigTag::FalAiChat | ChatConfigTag::FeatherlessAi
        | ChatConfigTag::FireworksAi | ChatConfigTag::FriendliaiChat | ChatConfigTag::GaladrielChat | ChatConfigTag::GdcGemini | ChatConfigTag::GenAiHubOrchestration
        | ChatConfigTag::GigaChat | ChatConfigTag::GithubChat | ChatConfigTag::GithubCopilot | ChatConfigTag::GoogleAiStudioGemini | ChatConfigTag::GradientAi
        | ChatConfigTag::GroqChat | ChatConfigTag::HerokuChat | ChatConfigTag::HostedVllmChat | ChatConfigTag::HuggingFaceChat | ChatConfigTag::HyperbolicChat
        | ChatConfigTag::IbmWatsonXAi | ChatConfigTag::IbmWatsonXChat | ChatConfigTag::InceptionChat | ChatConfigTag::InceptionTextCompletion | ChatConfigTag::JsonProvider
        | ChatConfigTag::LambdaAiChat | ChatConfigTag::LitellmProxyChat | ChatConfigTag::LlamaApi | ChatConfigTag::LlamafileChat | ChatConfigTag::LmStudioChat
        | ChatConfigTag::Maritalk | ChatConfigTag::MinimaxChat | ChatConfigTag::Mistral | ChatConfigTag::ModelScopeChat | ChatConfigTag::MoonshotChat
        | ChatConfigTag::MorphChat | ChatConfigTag::Nadir | ChatConfigTag::Nebius | ChatConfigTag::NlpCloud | ChatConfigTag::Novita
        | ChatConfigTag::Nscale | ChatConfigTag::NvidiaNim | ChatConfigTag::OciChat | ChatConfigTag::Ollama | ChatConfigTag::OllamaChat
        | ChatConfigTag::Oobabooga | ChatConfigTag::OpenAiGpt | ChatConfigTag::OpenAiGpt5 | ChatConfigTag::OpenAiLikeChat | ChatConfigTag::OpenAiOSeries
        | ChatConfigTag::OpenAiTextCompletion | ChatConfigTag::OpenAiUnknownModel | ChatConfigTag::Openrouter | ChatConfigTag::OvhCloudChat | ChatConfigTag::PerplexityChat
        | ChatConfigTag::Petals | ChatConfigTag::Predibase | ChatConfigTag::QwenAiPlatformChat | ChatConfigTag::QwenCloudChat | ChatConfigTag::RagFlow
        | ChatConfigTag::Replicate | ChatConfigTag::Sagemaker | ChatConfigTag::SagemakerChat | ChatConfigTag::SagemakerNova | ChatConfigTag::SailChat
        | ChatConfigTag::Sambanova | ChatConfigTag::Snowflake | ChatConfigTag::TencentChat | ChatConfigTag::TogetherAiChat | ChatConfigTag::Triton
        | ChatConfigTag::V0Chat | ChatConfigTag::VercelAiGateway | ChatConfigTag::VertexAiAi21 | ChatConfigTag::VertexAiAnthropic | ChatConfigTag::VertexAiGptOss
        | ChatConfigTag::VertexAiLlama3 | ChatConfigTag::VertexAiMistral | ChatConfigTag::VertexGemini | ChatConfigTag::Vllm | ChatConfigTag::VolcEngine
        | ChatConfigTag::Wandb | ChatConfigTag::XaiChat | ChatConfigTag::ZaiChat => None,
    }
}
