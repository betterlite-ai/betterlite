mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum A2AProviderConfig {
    PydanticAI,
    BedrockAgentCore,
    LangFlow,
    WatsonxOrchestrate,
}

// LiteLLM's A2AProviderConfigManager.get_provider_config: exact provider names, with agentcore only under bedrock
#[must_use]
pub fn get_provider_config(
    custom_llm_provider: Option<&str>,
    model: Option<&str>,
) -> Option<A2AProviderConfig> {
    match custom_llm_provider? {
        "pydantic_ai_agents" => Some(A2AProviderConfig::PydanticAI),
        "bedrock" if model.is_some_and(|name| name.contains("agentcore")) => {
            Some(A2AProviderConfig::BedrockAgentCore)
        }
        "langflow" => Some(A2AProviderConfig::LangFlow),
        "watsonx_orchestrate" => Some(A2AProviderConfig::WatsonxOrchestrate),
        _ => None,
    }
}
