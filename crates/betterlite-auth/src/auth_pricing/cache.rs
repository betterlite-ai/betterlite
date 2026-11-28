use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

// Router._zero_cost_cache: a plain dict that lives and dies with this router instance
#[derive(Debug, Default)]
pub struct ZeroCostCache {
    verdicts: Mutex<BTreeMap<String, bool>>,
}

impl ZeroCostCache {
    #[must_use]
    pub fn get(&self, model: &str) -> Option<bool> {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(model)
            .copied()
    }

    pub fn record(&self, model: &str, zero_cost: bool) {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(model.to_owned(), zero_cost);
    }

    // Router._invalidate_model_group_info_cache drops every verdict on an add, remove or upsert
    pub fn clear(&self) {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    #[must_use]
    pub fn entries(&self) -> BTreeMap<String, bool> {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
