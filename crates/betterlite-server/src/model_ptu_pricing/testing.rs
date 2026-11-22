use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_model_info::metadata::ModelMetadataSource;

use crate::model_ptu_pricing::catalog::LoadedCatalog;

// A case's cost map: a lookup records its key and finds the entry, unless the case marks the map unreachable.
pub struct CostMap {
    pub entries: BTreeMap<String, JsonObject>,
    pub broken: bool,
    pub seen: Mutex<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
#[error("the cost map is unreachable")]
pub struct Unreachable;

impl CostMap {
    #[must_use]
    pub const fn new(entries: BTreeMap<String, JsonObject>, broken: bool) -> Self {
        Self {
            entries,
            broken,
            seen: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn lookups(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelMetadataSource for CostMap {
    type Error = Unreachable;

    fn get_model_info(
        &self,
        model: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Unreachable>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        let found = if self.broken {
            Err(Unreachable)
        } else {
            Ok(self.entries.get(model).cloned())
        };
        ready(found)
    }
}

impl LoadedCatalog for CostMap {
    fn entry(&self, key: &str) -> Option<JsonObject> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        self.entries.get(key).cloned()
    }
}
