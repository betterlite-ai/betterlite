use std::sync::Arc;

use betterlite_auth::auth_cache::gateway_cache;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::{Clock as JwksClock, SystemClock};
use betterlite_auth::jwt_keys::http::ProxiedJwksHttp;
use betterlite_auth::jwt_keys::resolver::JwksResolver;
use betterlite_auth::jwt_keys::settings::JwtKeySettings;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::disk::Clock as DiskClock;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// betterlite-cache's MEMORY_MAX_SIZE_PER_ITEM_KIB, which is crate-private there
const CACHE_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

pub struct Slot {
    // None when the memory tier cannot be built; a JWT credential then keeps its own rejection path.
    pub resolver: Option<Arc<JwksResolver>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        let settings = JwtKeySettings::from_config(config);
        let fetch_clock: DiskClock = {
            let system = Arc::new(SystemClock);
            Arc::new(move || system.now())
        };
        let Some(ordinary) =
            LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, Arc::clone(&fetch_clock)).ok()
        else {
            return Self { resolver: None };
        };
        let Some(key_objects) = LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, fetch_clock).ok()
        else {
            return Self { resolver: None };
        };
        // the JWKS entries always carry their own ttl, so nothing here reads the management fallback
        let cache = gateway_cache(
            Arc::new(ordinary),
            Arc::new(key_objects),
            None,
            DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
                redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
            },
            0.0,
        );
        let cache_tier: Arc<dyn KeyCache> = Arc::new(cache);
        let http = Arc::new(ProxiedJwksHttp {
            client: shared.client.clone(),
        });
        let wall_clock: Arc<dyn JwksClock> = Arc::new(SystemClock);
        Self {
            resolver: Some(Arc::new(JwksResolver::new(
                Arc::new(settings),
                cache_tier,
                http,
                wall_clock,
            ))),
        }
    }
}
