use rstest::rstest;

use crate::provider_selector::{A2AProviderConfig, get_provider_config};

#[rstest]
#[case::pydantic_ai_agents(Some("pydantic_ai_agents"), None, Some(A2AProviderConfig::PydanticAI))]
#[case::pydantic_ai_agents_model(
    Some("pydantic_ai_agents"),
    Some("agent"),
    Some(A2AProviderConfig::PydanticAI)
)]
#[case::bedrock_agentcore(
    Some("bedrock"),
    Some("agentcore-runtime"),
    Some(A2AProviderConfig::BedrockAgentCore)
)]
#[case::bedrock_agentcore_substring(
    Some("bedrock"),
    Some("bedrock-with-agentcore-support"),
    Some(A2AProviderConfig::BedrockAgentCore)
)]
#[case::bedrock_case_sensitive(Some("bedrock"), Some("AgentCore"), None)]
#[case::bedrock_other_model(Some("bedrock"), Some("claude"), None)]
#[case::bedrock_without_model(Some("bedrock"), None, None)]
#[case::bedrock_empty_model(Some("bedrock"), Some(""), None)]
#[case::langflow(Some("langflow"), Some("agent"), Some(A2AProviderConfig::LangFlow))]
#[case::langflow_without_model(Some("langflow"), None, Some(A2AProviderConfig::LangFlow))]
#[case::watsonx(
    Some("watsonx_orchestrate"),
    Some("granite"),
    Some(A2AProviderConfig::WatsonxOrchestrate)
)]
#[case::unknown_provider(Some("openai"), Some("gpt-4"), None)]
#[case::no_provider(None, Some("agentcore"), None)]
#[case::empty_provider(Some(""), Some("agentcore"), None)]
#[case::provider_case_sensitive(Some("Pydantic_AI_agents"), None, None)]
fn selects_the_config_litellm_would(
    #[case] custom_llm_provider: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: Option<A2AProviderConfig>,
) {
    assert_eq!(
        get_provider_config(custom_llm_provider, model),
        expected,
        "provider {custom_llm_provider:?} with model {model:?}"
    );
}
