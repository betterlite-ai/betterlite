use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;

use super::source::{CallableTokenProvider, CredentialSources, SourceFailure};

pub const REFRESH_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(128).unwrap();

pub struct RefreshProviderCache<'data> {
    sources: &'data dyn CredentialSources,
    entries: LruCache<String, Arc<dyn CallableTokenProvider + 'data>>,
}

impl<'data> RefreshProviderCache<'data> {
    #[must_use]
    pub fn new(sources: &'data dyn CredentialSources) -> Self {
        Self {
            sources,
            entries: LruCache::new(REFRESH_CACHE_CAPACITY),
        }
    }

    pub fn provider(
        &mut self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'data>, SourceFailure> {
        if let Some(provider) = self.entries.get(scope) {
            return Ok(Arc::clone(provider));
        }
        let sources: &'data dyn CredentialSources = self.sources;
        let provider = sources.sdk_refresh(scope)?;
        self.entries.put(scope.to_owned(), Arc::clone(&provider));
        Ok(provider)
    }
}
