use crate::catalog::ModelCatalog;
use crate::entry::ProviderProjection;

use super::ProviderModelSets;
use super::by_provider;
use super::provider;

impl ProviderModelSets {
    pub fn add_known_models(&mut self, catalog: &ModelCatalog) {
        for entry in catalog.entries.values() {
            self.add_entry(&entry.name, entry.fields.provider_projection());
        }
        self.models_by_provider = by_provider::build(&self.sets);
    }

    fn add_entry(&mut self, key: &str, provider: ProviderProjection<'_>) {
        let Some(set) = provider::set_for(provider.litellm_provider, provider.mode) else {
            return;
        };
        let Some(name) = provider::name_for(set, key) else {
            return;
        };
        self.sets.entry(set).or_default().insert(name);
    }
}
