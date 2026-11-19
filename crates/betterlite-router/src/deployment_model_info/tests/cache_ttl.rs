use std::sync::Arc;

use super::fixtures::{catalog, clock, dispatcher, limits, record, spec};
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_model_info::cache::{DISCOVERY_TTL_SECONDS, DiscoveryCache};
use crate::model_catalog_lookup::cache::CacheClear;

fn two_deployments() -> Arc<DeploymentCatalog> {
    catalog(vec![
        spec("group", r#"{"model":"openai/first"}"#, r#"{"id":"id0"}"#),
        spec("group", r#"{"model":"openai/second"}"#, r#"{"id":"id1"}"#),
    ])
}

#[test]
fn zero_capacity_stores_nothing() {
    let engine = two_deployments();
    let cache = DiscoveryCache::new(0, DISCOVERY_TTL_SECONDS, clock(&dispatcher(1000.0)));
    cache.store("id0", record(&engine, 0, limits(None, None, Some(4096))));
    assert!(cache.get("id0").is_none());
    assert!(cache.report().entries.is_empty());
}

#[test]
fn stale_entry_purged_when_storing_at_its_expiry() {
    let engine = two_deployments();
    let handle = dispatcher(1000.0);
    let cache = DiscoveryCache::new(2, DISCOVERY_TTL_SECONDS, clock(&handle));
    cache.store("id0", record(&engine, 0, limits(None, None, Some(4096))));
    *handle.lock().unwrap() = 1600.0;
    cache.store("id1", record(&engine, 1, limits(None, None, Some(4096))));
    assert_eq!(cache.report().entries, vec!["id1".to_owned()]);
}

#[test]
fn storing_again_keeps_the_expiry() {
    let engine = two_deployments();
    let handle = dispatcher(1000.0);
    let cache = DiscoveryCache::new(2, DISCOVERY_TTL_SECONDS, clock(&handle));
    cache.store("id0", record(&engine, 0, limits(None, None, Some(4096))));
    *handle.lock().unwrap() = 1200.0;
    cache.store("id0", record(&engine, 0, limits(None, None, Some(8192))));
    let report = cache.report();
    assert_eq!(report.expirations, vec![("id0".to_owned(), 1600.0)]);
    let stored = cache.get("id0").unwrap();
    assert_eq!(stored.limits.max_output_tokens, Some(8192));
}

#[test]
fn capacity_pressure_evicts_the_earliest_expiry() {
    let engine = two_deployments();
    let handle = dispatcher(1000.0);
    let cache = DiscoveryCache::new(1, DISCOVERY_TTL_SECONDS, clock(&handle));
    cache.store("id0", record(&engine, 0, limits(None, None, Some(4096))));
    *handle.lock().unwrap() = 1100.0;
    cache.store("id1", record(&engine, 1, limits(None, None, Some(4096))));
    assert_eq!(cache.report().entries, vec!["id1".to_owned()]);
    assert!(cache.get("id0").is_none());
}

#[test]
fn clear_removes_every_entry() {
    let engine = two_deployments();
    let cache = DiscoveryCache::for_deployments(2);
    cache.store("id0", record(&engine, 0, limits(None, None, Some(4096))));
    assert!(cache.get("id0").is_some());
    cache.clear();
    assert!(cache.get("id0").is_none());
    assert!(cache.report().entries.is_empty());
}
