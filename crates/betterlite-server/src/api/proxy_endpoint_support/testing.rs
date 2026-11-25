use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_router::model_catalog_lookup::error::LookupError;

use super::ModelNameCatalog;

// A catalog a case fills with names the predicate tests, counting the lookups it makes.
pub struct ScriptedCatalog {
    pub names: Vec<String>,
    pub calls: AtomicUsize,
}

impl ModelNameCatalog for ScriptedCatalog {
    fn model_names(&self) -> Result<Vec<String>, LookupError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.names.clone())
    }
}
