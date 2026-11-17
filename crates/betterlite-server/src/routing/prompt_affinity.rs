use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache};
use betterlite_config::sections::prompt_affinity as section;
use betterlite_logging::logging_metadata::base64::{Base64Error, Base64Truncator};
use betterlite_models::decisions::Json;
use betterlite_router::prompt_affinity::cache::{PIN_TTL_SECONDS, PinCache};
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum PromptAffinityError {
    #[error("MAX_BASE64_LENGTH_FOR_LOGGING is not a usable placeholder length")]
    Base64(#[from] Base64Error),
}

pub struct PromptAffinitySettings {
    pub enabled: bool,
    pub truncator: Base64Truncator,
}

// Router.add_optional_pre_call_checks builds the check off its config list, and the placeholders off the environment
pub fn settings(
    section: &section::Settings,
) -> Result<PromptAffinitySettings, PromptAffinityError> {
    Ok(PromptAffinitySettings {
        enabled: section.prompt_caching,
        truncator: Base64Truncator::from_env(section.maximum_base64_length.as_deref())?,
    })
}

pub struct DualCachePins<Memory: MemoryCacheOps> {
    pub cache: Arc<DualCache<Memory>>,
}

// PromptCachingCache: the pin write carries PROMPT_CACHE_PIN_TTL_SECONDS, and the read walks the keys newest first
#[async_trait]
impl<Memory: MemoryCacheOps + 'static> PinCache for DualCachePins<Memory> {
    async fn store(&self, key: &str, value: &Json) {
        let options = CacheOptions {
            ttl: Some(PIN_TTL_SECONDS),
            local_only: false,
            refresh_ttl: false,
        };
        self.cache.async_set_cache(key, value, options).await;
    }

    async fn read(&self, keys: &[String]) -> Vec<Option<Json>> {
        let mut values = Vec::with_capacity(keys.len());
        for key in keys {
            values.push(self.cache.async_get_cache(key, false).await);
        }
        values
    }
}

#[must_use]
pub fn pin_cache<Memory: MemoryCacheOps + 'static>(
    cache: Arc<DualCache<Memory>>,
) -> Arc<dyn PinCache> {
    Arc::new(DualCachePins { cache })
}
