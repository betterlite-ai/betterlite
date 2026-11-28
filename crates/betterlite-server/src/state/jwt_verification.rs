use std::sync::Arc;

use betterlite_auth::auth_cache::gateway_cache;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::{Clock as JwksClock, SystemClock};
use betterlite_auth::jwt_verification::settings::JwtVerificationSettings;
use betterlite_auth::jwt_verification::userinfo::ProxiedUserinfoHttp;
use betterlite_auth::jwt_verification::verifier::JwtVerifier;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::disk::Clock as DiskClock;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// betterlite-cache's MEMORY_MAX_SIZE_PER_ITEM_KIB, which is crate-private there
const CACHE_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

// A verifier exists only when some JWT input is configured, so a proxy without JWT auth answers as before.
pub struct Slot {
    pub verifier: Option<Arc<JwtVerifier>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        let settings = JwtVerificationSettings::from_config(config);
        if !settings.is_configured() {
            return Self { verifier: None };
        }
        let fetch_clock: DiskClock = {
            let system = Arc::new(SystemClock);
            Arc::new(move || system.now())
        };
        let (Some(ordinary), Some(userinfo)) = (
            LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, Arc::clone(&fetch_clock)).ok(),
            LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, fetch_clock).ok(),
        ) else {
            return Self { verifier: None };
        };
        // the userinfo entries always carry their own ttl, so nothing here reads the management fallback
        let cache_tier: Arc<dyn KeyCache> = Arc::new(gateway_cache(
            Arc::new(ordinary),
            Arc::new(userinfo),
            None,
            DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
                redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
            },
            0.0,
        ));
        let http = Arc::new(ProxiedUserinfoHttp {
            client: shared.client.clone(),
        });
        let wall_clock: Arc<dyn JwksClock> = Arc::new(SystemClock);
        Self {
            verifier: Some(Arc::new(JwtVerifier::new(
                Arc::new(settings),
                cache_tier,
                http,
                wall_clock,
            ))),
        }
    }
}
