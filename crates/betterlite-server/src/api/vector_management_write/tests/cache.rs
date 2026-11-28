#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;

use super::support::memory_store;
use crate::api::vector_management_write::cache::{RegistryCache, VectorStoreCache as _};

#[test]
fn registry_cache_adds_and_finds_a_store() {
    let cache = RegistryCache(Arc::new(Mutex::new(VectorStoreRegistry::default())));
    assert!(cache.find("vs_a").is_none());
    cache
        .add(memory_store("vs_a"))
        .expect("the registry takes the store");
    let found = cache.find("vs_a").expect("the registry carries the store");
    assert_eq!(Some("vs_a".to_owned()), found.vector_store_id);
}

#[test]
fn registry_cache_updates_and_removes_a_store() {
    let cache = RegistryCache(Arc::new(Mutex::new(VectorStoreRegistry::default())));
    cache
        .add(memory_store("vs_a"))
        .expect("the registry takes the store");
    let mut changed = memory_store("vs_a");
    changed.vector_store_description = Some("changed".to_owned());
    cache
        .update("vs_a", changed)
        .expect("the registry takes the update");
    let found = cache.find("vs_a").expect("the registry carries the store");
    assert_eq!(Some("changed".to_owned()), found.vector_store_description);
    cache
        .update("vs_b", memory_store("vs_b"))
        .expect("the registry takes a store it does not hold");
    assert!(cache.find("vs_b").is_some());
    cache
        .remove("vs_a")
        .expect("the registry takes the removal");
    assert!(cache.find("vs_a").is_none());
}
