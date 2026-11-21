#![cfg(test)]

use std::ptr;

use super::cache::incremental_scan_cache;
use super::support::MemoryCache;

#[test]
fn the_shared_proxy_cache_wins_when_the_proxy_has_one() {
    let shared = MemoryCache::default();
    let local = MemoryCache::default();
    let chosen = incremental_scan_cache(Some(&shared), &local);
    assert!(ptr::eq(chosen, &shared));
}

#[test]
fn the_process_local_cache_is_the_fallback() {
    let local = MemoryCache::default();
    let chosen = incremental_scan_cache(None, &local);
    assert!(ptr::eq(chosen, &local));
}
