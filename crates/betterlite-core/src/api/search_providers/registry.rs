pub mod config;
pub mod error;
pub mod names;
mod providers;
mod tests;
use crate::search::SearchConfigLookup;
use betterlite_config::loader::EnvironmentLookup;
use config::SearchConfig;
use names::SearchProviderName;
use std::str::FromStr as _;
pub struct SearchRegistry<'env> {
    pub environment: &'env dyn EnvironmentLookup,
}
// ProviderConfigManager.get_provider_search_config: a new config per call, None for an unmapped name
impl<'env> SearchConfigLookup for SearchRegistry<'env> {
    type Config = SearchConfig<'env>;
    fn search_config(&self, provider: &str) -> Option<SearchConfig<'env>> {
        let name = SearchProviderName::from_str(provider).ok()?;
        Some(providers::build(name, self.environment))
    }
}
