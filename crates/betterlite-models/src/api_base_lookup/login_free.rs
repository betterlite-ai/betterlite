use crate::identity::provider_id::ProviderId;

use super::{ApiBaseHooks, LoginFreeProvider};

// Every ProviderId without a login-free base, so a variant added to the enum fails this build.
macro_rules! other_providers {
    () => {
        ProviderId::Openai
            | ProviderId::OpenaiLike
            | ProviderId::JinaAi
            | ProviderId::Xai
            | ProviderId::Zai
            | ProviderId::CustomOpenai
            | ProviderId::TextCompletionOpenai
            | ProviderId::Cohere
            | ProviderId::CohereChat
            | ProviderId::Clarifai
            | ProviderId::Anthropic
            | ProviderId::AnthropicText
            | ProviderId::Bytez
            | ProviderId::Replicate
            | ProviderId::Reducto
            | ProviderId::AwsTextract
            | ProviderId::Runwayml
            | ProviderId::AwsPolly
            | ProviderId::Transcribe
            | ProviderId::Huggingface
            | ProviderId::TogetherAi
            | ProviderId::Openrouter
            | ProviderId::Datarobot
            | ProviderId::VertexAi
            | ProviderId::VertexAiBeta
            | ProviderId::Gemini
            | ProviderId::Ai21
            | ProviderId::Baseten
            | ProviderId::BlackForestLabs
            | ProviderId::Azure
            | ProviderId::AzureText
            | ProviderId::AzureAi
            | ProviderId::Sagemaker
            | ProviderId::SagemakerChat
            | ProviderId::SagemakerNova
            | ProviderId::Bedrock
            | ProviderId::Vllm
            | ProviderId::NlpCloud
            | ProviderId::Petals
            | ProviderId::Oobabooga
            | ProviderId::Ollama
            | ProviderId::OllamaChat
            | ProviderId::Deepinfra
            | ProviderId::Perplexity
            | ProviderId::Typesafe
            | ProviderId::StrandsDecider
            | ProviderId::Mistral
            | ProviderId::Milvus
            | ProviderId::Groq
            | ProviderId::A2a
            | ProviderId::Gigachat
            | ProviderId::NvidiaNim
            | ProviderId::NvidiaRiva
            | ProviderId::Soniox
            | ProviderId::Cerebras
            | ProviderId::Nadir
            | ProviderId::Ai21Chat
            | ProviderId::Volcengine
            | ProviderId::Codestral
            | ProviderId::TextCompletionCodestral
            | ProviderId::Dashscope
            | ProviderId::Qwencloud
            | ProviderId::QwenAiPlatform
            | ProviderId::Modelscope
            | ProviderId::Moonshot
            | ProviderId::Publicai
            | ProviderId::V0
            | ProviderId::Morph
            | ProviderId::LambdaAi
            | ProviderId::Inception
            | ProviderId::TextCompletionInception
            | ProviderId::Deepseek
            | ProviderId::Sambanova
            | ProviderId::Maritalk
            | ProviderId::Voyage
            | ProviderId::Cloudflare
            | ProviderId::Xinference
            | ProviderId::FireworksAi
            | ProviderId::Friendliai
            | ProviderId::FeatherlessAi
            | ProviderId::Watsonx
            | ProviderId::WatsonxText
            | ProviderId::Triton
            | ProviderId::Predibase
            | ProviderId::Databricks
            | ProviderId::Empower
            | ProviderId::Github
            | ProviderId::Ragflow
            | ProviderId::Compactifai
            | ProviderId::DockerModelRunner
            | ProviderId::Custom
            | ProviderId::LitellmProxy
            | ProviderId::HostedVllm
            | ProviderId::Tencent
            | ProviderId::Llamafile
            | ProviderId::LmStudio
            | ProviderId::Galadriel
            | ProviderId::Nebius
            | ProviderId::Infinity
            | ProviderId::Deepgram
            | ProviderId::Elevenlabs
            | ProviderId::Novita
            | ProviderId::AiohttpOpenai
            | ProviderId::Langfuse
            | ProviderId::Humanloop
            | ProviderId::Topaz
            | ProviderId::SapGenerativeAiHub
            | ProviderId::Assemblyai
            | ProviderId::AzureSpeech
            | ProviderId::CharityEngine
            | ProviderId::Snowflake
            | ProviderId::GradientAi
            | ProviderId::Llama
            | ProviderId::Nscale
            | ProviderId::PgVector
            | ProviderId::S3Vectors
            | ProviderId::Valkey
            | ProviderId::Mongodb
            | ProviderId::Helicone
            | ProviderId::Hyperbolic
            | ProviderId::Recraft
            | ProviderId::FalAi
            | ProviderId::Stability
            | ProviderId::Heroku
            | ProviderId::Aiml
            | ProviderId::Cometapi
            | ProviderId::Oci
            | ProviderId::AutoRouter
            | ProviderId::VercelAiGateway
            | ProviderId::Edenai
            | ProviderId::Dotprompt
            | ProviderId::Manus
            | ProviderId::Wandb
            | ProviderId::Ovhcloud
            | ProviderId::Scaleway
            | ProviderId::Lemonade
            | ProviderId::AmazonNova
            | ProviderId::A2aAgent
            | ProviderId::Langgraph
            | ProviderId::Langflow
            | ProviderId::Minimax
            | ProviderId::Synthetic
            | ProviderId::Apertis
            | ProviderId::Nanogpt
            | ProviderId::Poe
            | ProviderId::Chutes
            | ProviderId::Abliteration
            | ProviderId::Llamagate
            | ProviderId::Gmi
            | ProviderId::Sarvam
            | ProviderId::Aihubmix
            | ProviderId::Crusoe
            | ProviderId::Empiriolabs
            | ProviderId::Veniceai
            | ProviderId::Neosantara
            | ProviderId::Parasail
            | ProviderId::XiaomiMimo
            | ProviderId::Tensormesh
            | ProviderId::Libertai
            | ProviderId::Pinstripes
            | ProviderId::Cognition
            | ProviderId::Cortecs
            | ProviderId::ScxAi
            | ProviderId::Prism
            | ProviderId::Darkbloom
            | ProviderId::Meta
            | ProviderId::Sail
            | ProviderId::LitellmAgent
            | ProviderId::Cursor
            | ProviderId::BedrockMantle
            | ProviderId::Gdc
    };
}

// Ports _api_base_without_login: only these two providers have a login-free base in LiteLLM.
pub(super) fn api_base(declared: ProviderId, hooks: &dyn ApiBaseHooks) -> Option<String> {
    match declared {
        ProviderId::GithubCopilot => hooks.login_free_api_base(LoginFreeProvider::GithubCopilot),
        ProviderId::Chatgpt => hooks.login_free_api_base(LoginFreeProvider::Chatgpt),
        // B209's set holds only the two providers above, so no other id can arrive here
        other_providers!() => None,
    }
}
