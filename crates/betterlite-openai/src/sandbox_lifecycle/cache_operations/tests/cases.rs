#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::{FixedClock, MockConfig, MockRegistry, MockToolRegistry, block_on, test_object};

#[cfg(test)]
const fn provider<'provider>(
    config: Option<&'provider MockConfig>,
    registry: &'provider MockRegistry,
    tool_registry: &'provider MockToolRegistry,
) -> SandboxProvider<'provider, MockConfig, MockRegistry, MockToolRegistry> {
    SandboxProvider {
        sandbox_config: config,
        registry,
        tool_registry,
        sandbox_tool_name: None,
    }
}

#[cfg(test)]
#[test]
fn get_cached_container_returns_the_cached_container_without_creating_a_new_one() {
    let config = MockConfig::failing("must not create");
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(Some(&config), &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let base = Instant::now();
    cache.entries.insert(
        "cache-key".to_owned(),
        CacheEntry {
            container: test_object([("id", json!("cached"))]),
            params: None,
            last_accessed: base,
            identity: None,
        },
    );
    let (container, _params) = block_on(get_or_create_container(
        &sandbox,
        &mut cache,
        Some("cache-key"),
        None,
        &FixedClock(base),
    ))
    .unwrap();
    assert_eq!(container, test_object([("id", json!("cached"))]));
}

#[cfg(test)]
#[test]
fn get_cached_container_updates_the_last_access_timestamp() {
    let config = MockConfig::succeeding(test_object([("id", json!("cached"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(Some(&config), &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let base = Instant::now();
    cache.entries.insert(
        "cache-key".to_owned(),
        CacheEntry {
            container: test_object([("id", json!("cached"))]),
            params: None,
            last_accessed: base,
            identity: None,
        },
    );
    let touched_at = base + Duration::from_secs(800);
    block_on(get_or_create_container(
        &sandbox,
        &mut cache,
        Some("cache-key"),
        None,
        &FixedClock(touched_at),
    ))
    .unwrap();
    let pruned_at = touched_at + Duration::from_secs(800);
    block_on(prune_expired_cache(
        &sandbox,
        &mut cache,
        &FixedClock(pruned_at),
    ));
    assert_eq!(cache.entries.len(), 1);
}

#[cfg(test)]
#[test]
fn create_new_container_creates_and_caches_when_the_cache_key_is_unseen() {
    let config = MockConfig::succeeding(test_object([("id", json!("fresh"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(Some(&config), &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let now = Instant::now();
    let (container, _params) = block_on(get_or_create_container(
        &sandbox,
        &mut cache,
        Some("cache-key"),
        Some("user-1"),
        &FixedClock(now),
    ))
    .unwrap();
    assert_eq!(container, test_object([("id", json!("fresh"))]));
    assert_eq!(cache.entries.len(), 1);
    let entry = &cache.entries["cache-key"];
    assert_eq!(entry.identity, Some("user-1".to_owned()));
    assert_eq!(entry.last_accessed, now);
}

#[cfg(test)]
#[test]
fn create_new_container_evicts_the_lru_session_entry_when_the_identity_is_at_the_cap() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("fresh"))]));
    let tool_registry = MockToolRegistry {
        resolved: Some(
            test_object([("sandbox_provider", json!("e2b"))])
                .as_object()
                .unwrap()
                .clone(),
        ),
    };
    let sandbox: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: Some("my-tool"),
        };
    let mut cache = SandboxCache::default();
    insert_session_entries(
        &mut cache,
        "key",
        "user-1",
        Instant::now(),
        SESSION_SCOPED_IDENTITY_CAP,
    );
    block_on(get_or_create_container(
        &sandbox,
        &mut cache,
        Some("key-new"),
        Some("user-1"),
        &FixedClock(Instant::now() + Duration::from_secs(1000)),
    ))
    .unwrap();
    assert_eq!(cache.entries.len(), SESSION_SCOPED_IDENTITY_CAP);
    assert!(!cache.entries.contains_key("key-0"));
    assert!(cache.entries.contains_key("key-new"));
    assert_eq!(
        registry.delete_calls.lock().unwrap().as_slice(),
        [test_object([("id", json!("key-0"))])]
    );
}

#[cfg(test)]
#[test]
fn create_new_container_does_not_cache_when_no_cache_key_is_given() {
    let config = MockConfig::succeeding(test_object([("id", json!("fresh"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(Some(&config), &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    block_on(get_or_create_container(
        &sandbox,
        &mut cache,
        None,
        None,
        &FixedClock(Instant::now()),
    ))
    .unwrap();
    assert_eq!(cache.entries.len(), 0);
}

#[cfg(test)]
fn insert_session_entry(
    cache: &mut SandboxCache,
    key: &str,
    identity: &str,
    last_accessed: Instant,
) {
    cache.entries.insert(
        key.to_owned(),
        CacheEntry {
            container: test_object([("id", json!(key))]),
            params: Some(SandboxToolParams {
                sandbox_provider: "e2b".to_owned(),
                api_key: None,
                api_base: None,
            }),
            last_accessed,
            identity: Some(identity.to_owned()),
        },
    );
}

#[cfg(test)]
fn insert_session_entries(
    cache: &mut SandboxCache,
    prefix: &str,
    identity: &str,
    base: Instant,
    count: usize,
) {
    for offset in 0..count {
        let seconds = u64::try_from(offset).unwrap();
        insert_session_entry(
            cache,
            &format!("{prefix}-{offset}"),
            identity,
            base.checked_add(Duration::from_secs(seconds)).unwrap(),
        );
    }
}

#[cfg(test)]
#[test]
fn evict_lru_session_keeps_entries_below_the_cap() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(None::<&MockConfig>, &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    insert_session_entries(
        &mut cache,
        "key",
        "user-1",
        Instant::now(),
        SESSION_SCOPED_IDENTITY_CAP - 1,
    );
    block_on(evict_lru_session_if_over_cap(
        &sandbox, &mut cache, "user-1",
    ));
    assert_eq!(cache.entries.len(), SESSION_SCOPED_IDENTITY_CAP - 1);
}

#[cfg(test)]
#[test]
fn evict_lru_session_removes_the_least_recently_accessed_entry_at_the_cap() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(None::<&MockConfig>, &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    insert_session_entries(
        &mut cache,
        "key",
        "user-1",
        Instant::now(),
        SESSION_SCOPED_IDENTITY_CAP,
    );
    block_on(evict_lru_session_if_over_cap(
        &sandbox, &mut cache, "user-1",
    ));
    assert_eq!(cache.entries.len(), SESSION_SCOPED_IDENTITY_CAP - 1);
    assert!(!cache.entries.contains_key("key-0"));
    assert!(cache.entries.contains_key("key-9"));
    assert_eq!(
        registry.delete_calls.lock().unwrap().as_slice(),
        [test_object([("id", json!("key-0"))])]
    );
}

#[cfg(test)]
#[test]
fn evict_lru_session_ignores_entries_with_a_different_identity() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(None::<&MockConfig>, &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    insert_session_entries(
        &mut cache,
        "other",
        "user-2",
        Instant::now(),
        SESSION_SCOPED_IDENTITY_CAP,
    );
    block_on(evict_lru_session_if_over_cap(
        &sandbox, &mut cache, "user-1",
    ));
    assert_eq!(cache.entries.len(), SESSION_SCOPED_IDENTITY_CAP);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[cfg(test)]
#[test]
fn prune_expired_cache_removes_entries_older_than_the_ttl_and_deletes_them() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("stale"))]));
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(None::<&MockConfig>, &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let base = Instant::now();
    cache.entries.insert(
        "stale".to_owned(),
        CacheEntry {
            container: test_object([("id", json!("stale"))]),
            params: Some(SandboxToolParams {
                sandbox_provider: "e2b".to_owned(),
                api_key: None,
                api_base: None,
            }),
            last_accessed: base,
            identity: None,
        },
    );
    let now = base + Duration::from_secs(CACHE_TTL_SECONDS + 1);
    block_on(prune_expired_cache(&sandbox, &mut cache, &FixedClock(now)));
    assert_eq!(cache.entries.len(), 0);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 1);
}

#[cfg(test)]
#[test]
fn prune_expired_cache_keeps_entries_within_the_ttl() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(None::<&MockConfig>, &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let base = Instant::now();
    cache.entries.insert(
        "fresh".to_owned(),
        CacheEntry {
            container: test_object([("id", json!("fresh"))]),
            params: None,
            last_accessed: base,
            identity: None,
        },
    );
    let now = base + Duration::from_secs(CACHE_TTL_SECONDS - 1);
    block_on(prune_expired_cache(&sandbox, &mut cache, &FixedClock(now)));
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[cfg(test)]
#[test]
fn prune_expired_cache_keeps_an_entry_exactly_at_the_ttl_boundary() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(None::<&MockConfig>, &registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let base = Instant::now();
    cache.entries.insert(
        "at-boundary".to_owned(),
        CacheEntry {
            container: test_object([("id", json!("at-boundary"))]),
            params: None,
            last_accessed: base,
            identity: None,
        },
    );
    let now = base + Duration::from_secs(CACHE_TTL_SECONDS);
    block_on(prune_expired_cache(&sandbox, &mut cache, &FixedClock(now)));
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}
