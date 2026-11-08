use betterlite_models::identity::provider_id::ProviderId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponsesModelFacts {
    pub azure: AzureResponsesFacts,
    pub github_copilot_responses: bool,
    pub mantle: MantleResponses,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AzureResponsesFacts {
    pub supports_reasoning: bool,
    pub native_responses: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MantleResponses {
    #[default]
    Unsupported,
    Supported {
        use_openai_path: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ResponsesConfigTag {
    OpenAi,
    AzureOSeries,
    Azure,
    AzureAi,
    Xai,
    Sail,
    GithubCopilot,
    ChatGpt,
    LiteLlmProxy,
    VolcEngine,
    Manus,
    Perplexity,
    Databricks,
    OpenRouter,
    HostedVllm,
    FireworksAi,
    EdenAi,
    Bedrock(Option<String>),
    BedrockMantle(bool),
}

impl ResponsesConfigTag {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAIResponsesAPIConfig",
            Self::AzureOSeries => "AzureOpenAIOSeriesResponsesAPIConfig",
            Self::Azure => "AzureOpenAIResponsesAPIConfig",
            Self::AzureAi => "AzureAIResponsesAPIConfig",
            Self::Xai => "XAIResponsesAPIConfig",
            Self::Sail => "SailResponsesAPIConfig",
            Self::GithubCopilot => "GithubCopilotResponsesAPIConfig",
            Self::ChatGpt => "ChatGPTResponsesAPIConfig",
            Self::LiteLlmProxy => "LiteLLMProxyResponsesAPIConfig",
            Self::VolcEngine => "VolcEngineResponsesAPIConfig",
            Self::Manus => "ManusResponsesAPIConfig",
            Self::Perplexity => "PerplexityResponsesConfig",
            Self::Databricks => "DatabricksResponsesAPIConfig",
            Self::OpenRouter => "OpenRouterResponsesAPIConfig",
            Self::HostedVllm => "HostedVLLMResponsesAPIConfig",
            Self::FireworksAi => "FireworksAIResponsesAPIConfig",
            Self::EdenAi => "EdenAIResponsesAPIConfig",
            Self::Bedrock(_) => "BedrockOpenAIResponsesConfig",
            Self::BedrockMantle(_) => "BedrockMantleResponsesAPIConfig",
        }
    }
}

const DIRECT_TAGS: &[(ProviderId, ResponsesConfigTag)] = &[
    (ProviderId::Openai, ResponsesConfigTag::OpenAi),
    (ProviderId::Xai, ResponsesConfigTag::Xai),
    (ProviderId::Sail, ResponsesConfigTag::Sail),
    (ProviderId::Chatgpt, ResponsesConfigTag::ChatGpt),
    (ProviderId::LitellmProxy, ResponsesConfigTag::LiteLlmProxy),
    (ProviderId::Volcengine, ResponsesConfigTag::VolcEngine),
    (ProviderId::Manus, ResponsesConfigTag::Manus),
    (ProviderId::Perplexity, ResponsesConfigTag::Perplexity),
    (ProviderId::Openrouter, ResponsesConfigTag::OpenRouter),
    (ProviderId::HostedVllm, ResponsesConfigTag::HostedVllm),
    (ProviderId::FireworksAi, ResponsesConfigTag::FireworksAi),
    (ProviderId::Edenai, ResponsesConfigTag::EdenAi),
];

fn select_azure_responses(model: Option<&str>, facts: AzureResponsesFacts) -> ResponsesConfigTag {
    let Some(name) = model else {
        return ResponsesConfigTag::Azure;
    };
    let lowered = name.to_lowercase();
    let reasoning_only = facts.supports_reasoning && !lowered.contains("gpt");
    if lowered.contains("o_series") || reasoning_only {
        ResponsesConfigTag::AzureOSeries
    } else {
        ResponsesConfigTag::Azure
    }
}

#[must_use]
pub fn select_responses_config(
    provider: Option<ProviderId>,
    model: Option<&str>,
    facts: &ResponsesModelFacts,
) -> Option<ResponsesConfigTag> {
    let provider_id = provider?;
    if provider_id == ProviderId::Azure {
        return Some(select_azure_responses(model, facts.azure));
    }
    if provider_id == ProviderId::AzureAi {
        return facts
            .azure
            .native_responses
            .then_some(ResponsesConfigTag::AzureAi);
    }
    if provider_id == ProviderId::GithubCopilot {
        return (model.is_none() || facts.github_copilot_responses)
            .then_some(ResponsesConfigTag::GithubCopilot);
    }
    if provider_id == ProviderId::Databricks {
        let gpt = model.is_some_and(|name| name.to_lowercase().contains("gpt"));
        return gpt.then_some(ResponsesConfigTag::Databricks);
    }
    if provider_id == ProviderId::Bedrock {
        return Some(ResponsesConfigTag::Bedrock(model.map(str::to_owned)));
    }
    if provider_id == ProviderId::BedrockMantle {
        let MantleResponses::Supported { use_openai_path } = facts.mantle else {
            return None;
        };
        model?;
        return Some(ResponsesConfigTag::BedrockMantle(use_openai_path));
    }
    DIRECT_TAGS
        .iter()
        .find_map(|(row, tag)| (*row == provider_id).then(|| tag.clone()))
}
