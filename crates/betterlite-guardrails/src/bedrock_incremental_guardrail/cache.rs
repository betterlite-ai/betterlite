#[must_use]
pub const fn incremental_scan_cache<'cache, Store>(
    shared: Option<&'cache Store>,
    process_local: &'cache Store,
) -> &'cache Store {
    // LiteLLM's _incremental_scan_cache: the proxy's shared cache wins, else the process-local one.
    match shared {
        Some(cache) => cache,
        None => process_local,
    }
}
