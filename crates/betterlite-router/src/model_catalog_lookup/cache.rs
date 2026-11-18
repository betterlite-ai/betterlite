use std::sync::{Mutex, PoisonError};

// A cache a catalog edit invalidates. The service that fills one owns its storage and implements this.
pub trait CacheClear {
    fn clear(&self);
}

// Router._invalidate_model_group_info_cache and Router._invalidate_access_groups_cache, the two invalidation scopes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalidation {
    ModelMetadata,
    AccessGroups,
}

// The caches kept beside the catalog: model group info, deployment info, zero-cost verdicts, routing-group rows and access groups.
#[derive(Clone, Copy)]
pub struct CatalogCaches<'store> {
    pub model_group_info: &'store dyn CacheClear,
    pub deployment_model_info: &'store dyn CacheClear,
    pub zero_cost: &'store dyn CacheClear,
    pub routing_group_rows: &'store dyn CacheClear,
    pub access_groups: &'store dyn CacheClear,
}

impl CatalogCaches<'_> {
    // _invalidate_model_group_info_cache clears the metadata caches and the routing-group rows; _invalidate_access_groups_cache clears only the access groups.
    pub fn invalidate(&self, invalidation: Invalidation) {
        match invalidation {
            Invalidation::ModelMetadata => {
                let metadata = [
                    self.model_group_info,
                    self.deployment_model_info,
                    self.zero_cost,
                    self.routing_group_rows,
                ];
                for cache in metadata {
                    cache.clear();
                }
            }
            Invalidation::AccessGroups => self.access_groups.clear(),
        }
    }
}

// One cached value: a clear drops it, so the next reader computes it again.
#[derive(Debug, Default)]
pub struct CacheSlot<T>(Mutex<Option<T>>);

impl<T: Clone> CacheSlot<T> {
    #[must_use]
    pub fn read(&self) -> Option<T> {
        let slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        slot.clone()
    }

    pub fn store(&self, value: T) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = Some(value);
    }
}

impl<T> CacheClear for CacheSlot<T> {
    fn clear(&self) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = None;
    }
}
