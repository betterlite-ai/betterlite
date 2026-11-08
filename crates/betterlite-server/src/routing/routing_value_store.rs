use betterlite_config::sections::local_ttl_cache as config;
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreError, StoreSettings};

mod tests;

pub fn local_store(
    section: &config::Settings,
    clock: Clock,
) -> Result<RoutingValueStore, StoreError> {
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: section.global_max_item_kib,
        },
        clock,
    )
}
