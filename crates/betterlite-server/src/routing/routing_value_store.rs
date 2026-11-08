use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::local_ttl_cache as config;
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreError, StoreSettings};

mod tests;

pub fn local_store(section: &config::Settings) -> Result<RoutingValueStore, StoreError> {
    let clock: Clock = Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    });
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: None,
            loaded_max_size_per_item_kib: section.global_max_item_kib,
        },
        clock,
    )
}
