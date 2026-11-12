use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};

#[derive(Default)]
pub struct BundledReasoningModels {
    catalog: BundledCatalog,
    cached: Option<Arc<BTreeSet<String>>>,
}

impl BundledReasoningModels {
    // the lru_cache(maxsize=1) equivalent: the bundled map is parsed and scanned once
    pub fn get(&mut self) -> Arc<BTreeSet<String>> {
        if let Some(cached) = &self.cached {
            return Arc::clone(cached);
        }
        let catalog = self.catalog.get();
        let models = Arc::new(openai_reasoning_models(catalog.as_ref()));
        self.cached = Some(Arc::clone(&models));
        models
    }
}

fn openai_reasoning_models(catalog: &ModelCatalog) -> BTreeSet<String> {
    catalog
        .entries
        .values()
        .filter(|entry| entry.fields.metadata.litellm_provider == "openai")
        .filter(|entry| entry.fields.capabilities.reasoning.supports_reasoning == Some(true))
        .map(|entry| entry.name.clone())
        .collect()
}
