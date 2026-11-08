#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use super::messages::{MessagesConfigTag, select_messages_config};
use super::passthrough::select_passthrough_config;
use super::passthrough::{BatchesConfigTag, PassthroughConfigTag, select_batches_config};
use super::realtime::{
    RealtimeConfigTag, RealtimeHttpConfigTag, select_realtime_config, select_realtime_http_config,
};
use super::responses::{
    AzureResponsesFacts, MantleResponses, ResponsesConfigTag, ResponsesModelFacts,
    select_responses_config,
};
use super::search_harness::{
    Harness, HarnessConfigTag, SearchConfigTag, SearchProvider, select_harness_config,
    select_search_config,
};
use super::skills_evals::{
    EvalsConfigTag, SkillsConfigTag, select_evals_config, select_skills_config,
};
use super::vector_stores::{
    VectorStoreConfigTag, VectorStoreFilesConfigTag, VectorStoresConfigTag,
    select_store_files_config, select_vector_store_config, select_vector_stores_config,
};

#[rstest]
#[case::anthropic(
    "claude-sonnet-4-6",
    ProviderId::Anthropic,
    Some(MessagesConfigTag::Anthropic)
)]
#[case::bedrock_claude("anthropic.claude-3-haiku", ProviderId::Bedrock, Some(MessagesConfigTag::Bedrock("anthropic.claude-3-haiku".to_owned())))]
#[case::bedrock_other("amazon.nova-lite", ProviderId::Bedrock, Some(MessagesConfigTag::Bedrock("amazon.nova-lite".to_owned())))]
#[case::bedrock_mantle_claude(
    "CLAUDE-3",
    ProviderId::BedrockMantle,
    Some(MessagesConfigTag::BedrockMantle)
)]
#[case::bedrock_mantle_other("nova", ProviderId::BedrockMantle, None)]
#[case::vertex_claude(
    "claude-3-opus",
    ProviderId::VertexAi,
    Some(MessagesConfigTag::VertexAi)
)]
#[case::vertex_other("gemini-2.5", ProviderId::VertexAi, None)]
#[case::azure_ai_claude("claude-3", ProviderId::AzureAi, Some(MessagesConfigTag::AzureAi))]
#[case::azure_ai_other("gpt-4o", ProviderId::AzureAi, None)]
#[case::minimax("minimax-m2", ProviderId::Minimax, Some(MessagesConfigTag::Minimax))]
#[case::outside_list("claude-3", ProviderId::Openai, None)]
fn messages_selection(
    #[case] model: &str,
    #[case] provider: ProviderId,
    #[case] expected: Option<MessagesConfigTag>,
) {
    assert_eq!(select_messages_config(model, provider), expected);
}

const NO_FACTS: ResponsesModelFacts = ResponsesModelFacts {
    azure: AzureResponsesFacts {
        supports_reasoning: false,
        native_responses: false,
    },
    github_copilot_responses: false,
    mantle: MantleResponses::Unsupported,
};

const REASONING: ResponsesModelFacts = ResponsesModelFacts {
    azure: AzureResponsesFacts {
        supports_reasoning: true,
        native_responses: false,
    },
    github_copilot_responses: false,
    mantle: MantleResponses::Unsupported,
};

const NATIVE: ResponsesModelFacts = ResponsesModelFacts {
    azure: AzureResponsesFacts {
        supports_reasoning: false,
        native_responses: true,
    },
    github_copilot_responses: true,
    mantle: MantleResponses::Supported {
        use_openai_path: true,
    },
};

#[rstest]
#[case::openai(ProviderId::Openai, ResponsesConfigTag::OpenAi)]
#[case::xai(ProviderId::Xai, ResponsesConfigTag::Xai)]
#[case::sail(ProviderId::Sail, ResponsesConfigTag::Sail)]
#[case::chatgpt(ProviderId::Chatgpt, ResponsesConfigTag::ChatGpt)]
#[case::litellm_proxy(ProviderId::LitellmProxy, ResponsesConfigTag::LiteLlmProxy)]
#[case::volcengine(ProviderId::Volcengine, ResponsesConfigTag::VolcEngine)]
#[case::manus(ProviderId::Manus, ResponsesConfigTag::Manus)]
#[case::perplexity(ProviderId::Perplexity, ResponsesConfigTag::Perplexity)]
#[case::openrouter(ProviderId::Openrouter, ResponsesConfigTag::OpenRouter)]
#[case::hosted_vllm(ProviderId::HostedVllm, ResponsesConfigTag::HostedVllm)]
#[case::fireworks_ai(ProviderId::FireworksAi, ResponsesConfigTag::FireworksAi)]
#[case::edenai(ProviderId::Edenai, ResponsesConfigTag::EdenAi)]
fn responses_direct_selection(#[case] provider: ProviderId, #[case] expected: ResponsesConfigTag) {
    assert_eq!(
        select_responses_config(Some(provider), Some("any-model"), &NO_FACTS),
        Some(expected)
    );
}

#[test]
fn responses_no_provider() {
    assert_eq!(
        select_responses_config(None, Some("any-model"), &NO_FACTS),
        None
    );
}

#[rstest]
#[case::azure_o_series(
    ProviderId::Azure,
    Some("o_series-preview"),
    NO_FACTS,
    Some(ResponsesConfigTag::AzureOSeries)
)]
#[case::azure_reasoning_non_gpt(
    ProviderId::Azure,
    Some("o1"),
    REASONING,
    Some(ResponsesConfigTag::AzureOSeries)
)]
#[case::azure_gpt_ignores_reasoning(
    ProviderId::Azure,
    Some("GPT-4o"),
    REASONING,
    Some(ResponsesConfigTag::Azure)
)]
#[case::azure_plain(
    ProviderId::Azure,
    Some("o1"),
    NO_FACTS,
    Some(ResponsesConfigTag::Azure)
)]
#[case::azure_no_model(ProviderId::Azure, None, REASONING, Some(ResponsesConfigTag::Azure))]
#[case::azure_ai_native(
    ProviderId::AzureAi,
    Some("m"),
    NATIVE,
    Some(ResponsesConfigTag::AzureAi)
)]
#[case::azure_ai_not_native(ProviderId::AzureAi, Some("m"), NO_FACTS, None)]
#[case::github_copilot_no_model(
    ProviderId::GithubCopilot,
    None,
    NO_FACTS,
    Some(ResponsesConfigTag::GithubCopilot)
)]
#[case::github_copilot_supported(
    ProviderId::GithubCopilot,
    Some("gpt-5.3-codex"),
    NATIVE,
    Some(ResponsesConfigTag::GithubCopilot)
)]
#[case::github_copilot_not_supported(
    ProviderId::GithubCopilot,
    Some("gpt-5.3-codex"),
    NO_FACTS,
    None
)]
#[case::databricks_gpt(
    ProviderId::Databricks,
    Some("databricks-gpt-4"),
    NO_FACTS,
    Some(ResponsesConfigTag::Databricks)
)]
#[case::databricks_other(ProviderId::Databricks, Some("llama-3"), NO_FACTS, None)]
#[case::databricks_no_model(ProviderId::Databricks, None, NO_FACTS, None)]
#[case::bedrock_with_model(ProviderId::Bedrock, Some("openai.gpt-5"), NO_FACTS, Some(ResponsesConfigTag::Bedrock(Some("openai.gpt-5".to_owned()))))]
#[case::bedrock_no_model(
    ProviderId::Bedrock,
    None,
    NO_FACTS,
    Some(ResponsesConfigTag::Bedrock(None))
)]
#[case::bedrock_mantle_supported(
    ProviderId::BedrockMantle,
    Some("gpt-5"),
    NATIVE,
    Some(ResponsesConfigTag::BedrockMantle(true))
)]
#[case::bedrock_mantle_unsupported(ProviderId::BedrockMantle, Some("gpt-5"), NO_FACTS, None)]
#[case::bedrock_mantle_no_model(ProviderId::BedrockMantle, None, NATIVE, None)]
#[case::outside_list(ProviderId::Groq, Some("m"), NO_FACTS, None)]
fn responses_fact_selection(
    #[case] provider: ProviderId,
    #[case] model: Option<&str>,
    #[case] facts: ResponsesModelFacts,
    #[case] expected: Option<ResponsesConfigTag>,
) {
    assert_eq!(
        select_responses_config(Some(provider), model, &facts),
        expected
    );
}

#[rstest]
#[case::bedrock(ProviderId::Bedrock, Some(PassthroughConfigTag::Bedrock))]
#[case::bedrock_mantle(ProviderId::BedrockMantle, Some(PassthroughConfigTag::BedrockMantle))]
#[case::vllm(ProviderId::Vllm, Some(PassthroughConfigTag::VLLM))]
#[case::hosted_vllm(ProviderId::HostedVllm, Some(PassthroughConfigTag::VLLM))]
#[case::azure(ProviderId::Azure, Some(PassthroughConfigTag::Azure))]
#[case::azure_ai(ProviderId::AzureAi, Some(PassthroughConfigTag::AzureAI))]
#[case::gigachat(ProviderId::Gigachat, Some(PassthroughConfigTag::GigaChat))]
#[case::watsonx(ProviderId::Watsonx, Some(PassthroughConfigTag::Watsonx))]
#[case::nvidia_nim(ProviderId::NvidiaNim, Some(PassthroughConfigTag::NvidiaNim))]
#[case::outside_list(ProviderId::Openai, None)]
fn passthrough_selection(
    #[case] provider: ProviderId,
    #[case] expected: Option<PassthroughConfigTag>,
) {
    assert_eq!(select_passthrough_config("any-model", provider), expected);
}

#[rstest]
#[case::bedrock(ProviderId::Bedrock, Some(BatchesConfigTag::Bedrock))]
#[case::mistral(ProviderId::Mistral, Some(BatchesConfigTag::Mistral))]
#[case::outside_list(ProviderId::Openai, None)]
fn batches_selection(#[case] provider: ProviderId, #[case] expected: Option<BatchesConfigTag>) {
    assert_eq!(select_batches_config("any-model", provider), expected);
}

#[rstest]
#[case::bedrock(ProviderId::Bedrock, Some(VectorStoreConfigTag::Bedrock))]
#[case::outside_list(ProviderId::Openai, None)]
fn vector_store_selection(
    #[case] provider: ProviderId,
    #[case] expected: Option<VectorStoreConfigTag>,
) {
    assert_eq!(select_vector_store_config(provider), expected);
}

#[rstest]
#[case::openai(ProviderId::Openai, None, Some(VectorStoresConfigTag::OpenAI))]
#[case::azure(ProviderId::Azure, None, Some(VectorStoresConfigTag::Azure))]
#[case::vertex_default(ProviderId::VertexAi, None, Some(VectorStoresConfigTag::VertexAI))]
#[case::vertex_rag(
    ProviderId::VertexAi,
    Some("rag_api"),
    Some(VectorStoresConfigTag::VertexAI)
)]
#[case::vertex_search(
    ProviderId::VertexAi,
    Some("search_api"),
    Some(VectorStoresConfigTag::VertexSearchAPI)
)]
#[case::vertex_unknown(ProviderId::VertexAi, Some("other_api"), None)]
#[case::bedrock(ProviderId::Bedrock, None, Some(VectorStoresConfigTag::Bedrock))]
#[case::pg_vector(ProviderId::PgVector, None, Some(VectorStoresConfigTag::PGVector))]
#[case::azure_ai(ProviderId::AzureAi, None, Some(VectorStoresConfigTag::AzureAI))]
#[case::milvus(ProviderId::Milvus, None, Some(VectorStoresConfigTag::Milvus))]
#[case::gemini(ProviderId::Gemini, None, Some(VectorStoresConfigTag::Gemini))]
#[case::ragflow(ProviderId::Ragflow, None, Some(VectorStoresConfigTag::RAGFlow))]
#[case::s3_vectors(ProviderId::S3Vectors, None, Some(VectorStoresConfigTag::S3Vectors))]
#[case::valkey(ProviderId::Valkey, None, Some(VectorStoresConfigTag::Valkey))]
#[case::mongodb(ProviderId::Mongodb, None, Some(VectorStoresConfigTag::MongoDB))]
#[case::outside_list(
    ProviderId::Openai,
    Some("rag_api"),
    Some(VectorStoresConfigTag::OpenAI)
)]
fn vector_stores_selection(
    #[case] provider: ProviderId,
    #[case] api_type: Option<&str>,
    #[case] expected: Option<VectorStoresConfigTag>,
) {
    assert_eq!(select_vector_stores_config(provider, api_type), expected);
}

#[test]
fn vector_stores_unknown_provider() {
    assert_eq!(select_vector_stores_config(ProviderId::Groq, None), None);
}

#[rstest]
#[case::openai(ProviderId::Openai, Some(VectorStoreFilesConfigTag::OpenAI))]
#[case::outside_list(ProviderId::Azure, None)]
fn vector_store_files_selection(
    #[case] provider: ProviderId,
    #[case] expected: Option<VectorStoreFilesConfigTag>,
) {
    assert_eq!(select_store_files_config(provider), expected);
}

#[rstest]
#[case::gemini(ProviderId::Gemini, Some(RealtimeConfigTag::Gemini))]
#[case::meta(ProviderId::Meta, Some(RealtimeConfigTag::Meta))]
#[case::outside_list(ProviderId::Openai, None)]
fn realtime_selection(#[case] provider: ProviderId, #[case] expected: Option<RealtimeConfigTag>) {
    assert_eq!(select_realtime_config("any-model", provider), expected);
}

#[rstest]
#[case::openai(ProviderId::Openai, Some(RealtimeHttpConfigTag::OpenAI))]
#[case::azure(ProviderId::Azure, Some(RealtimeHttpConfigTag::Azure))]
#[case::outside_list(ProviderId::Gemini, None)]
fn realtime_http_selection(
    #[case] provider: ProviderId,
    #[case] expected: Option<RealtimeHttpConfigTag>,
) {
    assert_eq!(select_realtime_http_config("any-model", provider), expected);
}

#[rstest]
#[case::anthropic(ProviderId::Anthropic, Some(SkillsConfigTag::Anthropic))]
#[case::outside_list(ProviderId::Openai, None)]
fn skills_selection(#[case] provider: ProviderId, #[case] expected: Option<SkillsConfigTag>) {
    assert_eq!(select_skills_config(provider), expected);
}

#[rstest]
#[case::openai(ProviderId::Openai, Some(EvalsConfigTag::OpenAI))]
#[case::outside_list(ProviderId::Anthropic, None)]
fn evals_selection(#[case] provider: ProviderId, #[case] expected: Option<EvalsConfigTag>) {
    assert_eq!(select_evals_config(provider), expected);
}

#[rstest]
#[case::perplexity(SearchProvider::Perplexity, SearchConfigTag::Perplexity)]
#[case::tavily(SearchProvider::Tavily, SearchConfigTag::Tavily)]
#[case::parallel_ai(SearchProvider::ParallelAi, SearchConfigTag::ParallelAI)]
#[case::exa_ai(SearchProvider::ExaAi, SearchConfigTag::ExaAI)]
#[case::brave(SearchProvider::Brave, SearchConfigTag::Brave)]
#[case::google_pse(SearchProvider::GooglePse, SearchConfigTag::GooglePSE)]
#[case::dataforseo(SearchProvider::DataForSeo, SearchConfigTag::DataForSEO)]
#[case::firecrawl(SearchProvider::Firecrawl, SearchConfigTag::Firecrawl)]
#[case::fastcrw(SearchProvider::FastCrw, SearchConfigTag::FastCRW)]
#[case::searxng(SearchProvider::SearXng, SearchConfigTag::SearXNG)]
#[case::linkup(SearchProvider::Linkup, SearchConfigTag::Linkup)]
#[case::duckduckgo(SearchProvider::DuckDuckGo, SearchConfigTag::DuckDuckGo)]
#[case::searchapi(SearchProvider::SearchApi, SearchConfigTag::SearchAPI)]
#[case::serper(SearchProvider::Serper, SearchConfigTag::Serper)]
#[case::you_com(SearchProvider::YouCom, SearchConfigTag::YouCom)]
#[case::apiserpent(SearchProvider::ApiSerpent, SearchConfigTag::APISerpent)]
#[case::tinyfish(SearchProvider::Tinyfish, SearchConfigTag::Tinyfish)]
#[case::agentcore(SearchProvider::AgentCore, SearchConfigTag::AgentCore)]
#[case::nimble(SearchProvider::Nimble, SearchConfigTag::Nimble)]
#[case::bing_grounding(SearchProvider::BingGrounding, SearchConfigTag::BingGrounding)]
fn search_selection(#[case] provider: SearchProvider, #[case] expected: SearchConfigTag) {
    assert_eq!(select_search_config(provider), expected);
}

#[rstest]
#[case::claude_code(Harness::ClaudeCode, HarnessConfigTag::ClaudeCode)]
#[case::codex(Harness::Codex, HarnessConfigTag::Codex)]
#[case::opencode(Harness::OpenCode, HarnessConfigTag::OpenCode)]
#[case::deep_agents(Harness::DeepAgents, HarnessConfigTag::DeepAgents)]
fn harness_selection(#[case] harness: Harness, #[case] expected: HarnessConfigTag) {
    assert_eq!(select_harness_config(harness), expected);
}

#[rstest]
#[case::messages(MessagesConfigTag::Anthropic, "AnthropicMessagesConfig")]
#[case::azure_ai(MessagesConfigTag::AzureAi, "AzureAnthropicMessagesConfig")]
fn messages_class_names(#[case] tag: MessagesConfigTag, #[case] name: &str) {
    assert_eq!(tag.class_name(), name);
}

#[rstest]
#[case::azure_oseries(
    ResponsesConfigTag::AzureOSeries,
    "AzureOpenAIOSeriesResponsesAPIConfig"
)]
#[case::perplexity(ResponsesConfigTag::Perplexity, "PerplexityResponsesConfig")]
#[case::bedrock(ResponsesConfigTag::Bedrock(None), "BedrockOpenAIResponsesConfig")]
fn responses_class_names(#[case] tag: ResponsesConfigTag, #[case] name: &str) {
    assert_eq!(tag.class_name(), name);
}
