#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    Perplexity,
    Tavily,
    ParallelAi,
    ExaAi,
    Brave,
    GooglePse,
    #[serde(rename = "dataforseo")]
    DataForSeo,
    Firecrawl,
    #[serde(rename = "fastcrw")]
    FastCrw,
    #[serde(rename = "searxng")]
    SearXng,
    Linkup,
    #[serde(rename = "duckduckgo")]
    DuckDuckGo,
    #[serde(rename = "searchapi")]
    SearchApi,
    Serper,
    YouCom,
    #[serde(rename = "apiserpent")]
    ApiSerpent,
    Tinyfish,
    #[serde(rename = "agentcore")]
    AgentCore,
    Nimble,
    #[serde(rename = "bing_grounding")]
    BingGrounding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchConfigTag {
    Perplexity,
    Tavily,
    ParallelAI,
    ExaAI,
    Brave,
    GooglePSE,
    DataForSEO,
    Firecrawl,
    FastCRW,
    SearXNG,
    Linkup,
    DuckDuckGo,
    SearchAPI,
    Serper,
    YouCom,
    APISerpent,
    Tinyfish,
    AgentCore,
    Nimble,
    BingGrounding,
}

#[must_use]
pub const fn select_search_config(provider: SearchProvider) -> SearchConfigTag {
    match provider {
        SearchProvider::Perplexity => SearchConfigTag::Perplexity,
        SearchProvider::Tavily => SearchConfigTag::Tavily,
        SearchProvider::ParallelAi => SearchConfigTag::ParallelAI,
        SearchProvider::ExaAi => SearchConfigTag::ExaAI,
        SearchProvider::Brave => SearchConfigTag::Brave,
        SearchProvider::GooglePse => SearchConfigTag::GooglePSE,
        SearchProvider::DataForSeo => SearchConfigTag::DataForSEO,
        SearchProvider::Firecrawl => SearchConfigTag::Firecrawl,
        SearchProvider::FastCrw => SearchConfigTag::FastCRW,
        SearchProvider::SearXng => SearchConfigTag::SearXNG,
        SearchProvider::Linkup => SearchConfigTag::Linkup,
        SearchProvider::DuckDuckGo => SearchConfigTag::DuckDuckGo,
        SearchProvider::SearchApi => SearchConfigTag::SearchAPI,
        SearchProvider::Serper => SearchConfigTag::Serper,
        SearchProvider::YouCom => SearchConfigTag::YouCom,
        SearchProvider::ApiSerpent => SearchConfigTag::APISerpent,
        SearchProvider::Tinyfish => SearchConfigTag::Tinyfish,
        SearchProvider::AgentCore => SearchConfigTag::AgentCore,
        SearchProvider::Nimble => SearchConfigTag::Nimble,
        SearchProvider::BingGrounding => SearchConfigTag::BingGrounding,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "deepagents")]
    DeepAgents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessConfigTag {
    ClaudeCode,
    Codex,
    OpenCode,
    DeepAgents,
}

#[must_use]
pub const fn select_harness_config(harness: Harness) -> HarnessConfigTag {
    match harness {
        Harness::ClaudeCode => HarnessConfigTag::ClaudeCode,
        Harness::Codex => HarnessConfigTag::Codex,
        Harness::OpenCode => HarnessConfigTag::OpenCode,
        Harness::DeepAgents => HarnessConfigTag::DeepAgents,
    }
}
