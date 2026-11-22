use super::config::{SearchConfig, SearchHooks};
use super::names::SearchProviderName;
use crate::api::search_providers::apiserpent::ApiserpentSearchConfig;
use crate::api::search_providers::brave::BraveSearchConfig;
use crate::api::search_providers::dataforseo::{DataForSeoSearchConfig, DataForSeoSearchSettings};
use crate::api::search_providers::duckduckgo::DuckDuckGoSearchConfig;
use crate::api::search_providers::exa_ai::{ExaAiSearchConfig, ExaAiSearchSettings};
use crate::api::search_providers::fastcrw::FastcrwSearch;
use crate::api::search_providers::firecrawl::FirecrawlSearch;
use crate::api::search_providers::nimble::NimbleSearch;
use crate::api::search_providers::parallel_ai::ParallelAiSearch;
use crate::api::search_providers::serper::{SerperSearchConfig, SerperSearchSettings};
use crate::api::search_providers::tavily::TavilySearch;
use crate::api::search_providers::tinyfish::TinyfishSearch;
use crate::api::search_providers::you_com::YouComSearch;
use adapter::Adapter;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::perplexity_search::PerplexitySearch;
use loaded::{ReloadedFlavor, ReloadedProvider};
use std::sync::Mutex;
use url::{UrlKeyedFlavor, UrlKeyedProvider};
mod adapter;
mod loaded;
mod shared;
mod url;
pub(super) fn build<'env>(
    name: SearchProviderName,
    environment: &'env dyn EnvironmentLookup,
) -> SearchConfig<'env> {
    let hooks: Box<dyn SearchHooks + 'env> = match name {
        SearchProviderName::Perplexity => Box::new(Adapter::new(PerplexitySearch { environment })),
        SearchProviderName::Tavily => Box::new(Adapter::new(TavilySearch::load(environment))),
        SearchProviderName::ParallelAi => {
            Box::new(Adapter::new(ParallelAiSearch::load(environment)))
        }
        SearchProviderName::ExaAi => Box::new(Adapter::new(ExaAiSearchConfig {
            settings: ExaAiSearchSettings::load(environment),
        })),
        SearchProviderName::Brave => {
            Box::new(Adapter::new(BraveSearchConfig::load(environment, None)))
        }
        SearchProviderName::GooglePse => Box::new(UrlKeyedProvider {
            environment,
            flavor: Mutex::new(UrlKeyedFlavor::GooglePse(None)),
        }),
        SearchProviderName::DataForSeo => Box::new(Adapter::new(DataForSeoSearchConfig {
            settings: DataForSeoSearchSettings::load(environment),
        })),
        SearchProviderName::Firecrawl => Box::new(Adapter::new(FirecrawlSearch::load(environment))),
        SearchProviderName::Fastcrw => Box::new(Adapter::new(FastcrwSearch::load(environment))),
        SearchProviderName::Searxng => Box::new(ReloadedProvider {
            environment,
            flavor: ReloadedFlavor::Searxng,
            body: Mutex::new(None),
        }),
        SearchProviderName::Linkup => Box::new(ReloadedProvider {
            environment,
            flavor: ReloadedFlavor::Linkup,
            body: Mutex::new(None),
        }),
        SearchProviderName::DuckDuckGo => {
            Box::new(Adapter::new(DuckDuckGoSearchConfig::load(environment)))
        }
        SearchProviderName::SearchApi => Box::new(UrlKeyedProvider {
            environment,
            flavor: Mutex::new(UrlKeyedFlavor::SearchApi(None)),
        }),
        SearchProviderName::Serper => Box::new(Adapter::new(SerperSearchConfig {
            settings: SerperSearchSettings::load(environment),
        })),
        SearchProviderName::YouCom => Box::new(Adapter::new(YouComSearch::load(environment))),
        SearchProviderName::Apiserpent => {
            Box::new(Adapter::new(ApiserpentSearchConfig::load(environment)))
        }
        SearchProviderName::Tinyfish => Box::new(Adapter::new(TinyfishSearch::load(environment))),
        SearchProviderName::Nimble => Box::new(Adapter::new(NimbleSearch::load(environment))),
    };
    SearchConfig { name, hooks }
}
