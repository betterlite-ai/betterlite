use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};

use lru::LruCache;

use crate::resolution::{ResolutionContext, ResolutionRequest, resolve};

mod tests;

const CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

// A cache belongs to one resolution context, so a config reload builds a new cache instead of reusing answers
#[derive(Debug)]
pub struct ProviderCache {
    entries: Mutex<LruCache<String, Option<String>>>,
}

impl Default for ProviderCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(LruCache::new(CACHE_CAPACITY)),
        }
    }
}

impl ProviderCache {
    pub fn qualified(&self, model: &str, context: &ResolutionContext<'_>) -> Option<String> {
        let cached = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(model)
            .cloned();
        if let Some(resolved) = cached {
            return resolved;
        }
        let resolved = provider_qualified(model, context);
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .put(model.to_owned(), resolved.clone());
        resolved
    }

    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

// The name litellm itself resolves the model to, or None when no provider claims it
fn provider_qualified(model: &str, context: &ResolutionContext<'_>) -> Option<String> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    let resolved = resolve(&request, context).ok()?;
    let provider = resolved.provider.slug();
    if provider.is_empty() || resolved.model.is_empty() {
        return None;
    }
    Some(format!("{provider}/{}", resolved.model))
}
