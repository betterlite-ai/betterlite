use strum::{EnumIter, EnumString};
// the SearchProviders values LiteLLM maps to a search config, in LiteLLM's declaration order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, EnumString)]
pub enum SearchProviderName {
    #[strum(serialize = "perplexity")]
    Perplexity,
    #[strum(serialize = "tavily")]
    Tavily,
    #[strum(serialize = "parallel_ai")]
    ParallelAi,
    #[strum(serialize = "exa_ai")]
    ExaAi,
    #[strum(serialize = "brave")]
    Brave,
    #[strum(serialize = "google_pse")]
    GooglePse,
    #[strum(serialize = "dataforseo")]
    DataForSeo,
    #[strum(serialize = "firecrawl")]
    Firecrawl,
    #[strum(serialize = "fastcrw")]
    Fastcrw,
    #[strum(serialize = "searxng")]
    Searxng,
    #[strum(serialize = "linkup")]
    Linkup,
    #[strum(serialize = "duckduckgo")]
    DuckDuckGo,
    #[strum(serialize = "searchapi")]
    SearchApi,
    #[strum(serialize = "serper")]
    Serper,
    #[strum(serialize = "you_com")]
    YouCom,
    #[strum(serialize = "apiserpent")]
    Apiserpent,
    #[strum(serialize = "tinyfish")]
    Tinyfish,
    #[strum(serialize = "nimble")]
    Nimble,
}
impl SearchProviderName {
    // the provider config's ui_friendly_name, which is what LiteLLM reports to a client
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Perplexity => "Perplexity",
            Self::Tavily => "Tavily",
            Self::ParallelAi => "Parallel AI",
            Self::ExaAi => "Exa AI",
            Self::Brave => "Brave Search",
            Self::GooglePse => "Google PSE",
            Self::DataForSeo => "DataForSEO",
            Self::Firecrawl => "Firecrawl",
            Self::Fastcrw => "fastCRW",
            Self::Searxng => "SearXNG",
            Self::Linkup => "Linkup",
            Self::DuckDuckGo => "DuckDuckGo",
            Self::SearchApi => "SearchAPI.io (Google Search)",
            Self::Serper => "Serper",
            Self::YouCom => "You.com",
            Self::Apiserpent => "APISerpent",
            Self::Tinyfish => "TinyFish",
            Self::Nimble => "Nimble",
        }
    }
}
