use std::convert::Infallible;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use lru::LruCache;

use crate::model_catalog_lookup::cache::CacheClear;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_info::aggregate::get_model_group_info;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_info::sources::{DeploymentInfo, GroupSources, ProviderResolver};

// Router._cached_get_model_group_info: functools.lru_cache over get_model_group_info, storing nothing at a maxsize of 0.
#[derive(Debug)]
pub struct GroupInfoCache {
    entries: Mutex<Option<LruCache<String, Option<ModelGroupInfo>>>>,
    generation: AtomicU64,
}

impl GroupInfoCache {
    #[must_use]
    pub fn new(capacity: i64) -> Self {
        let size = usize::try_from(capacity).ok().and_then(NonZeroUsize::new);
        Self {
            entries: Mutex::new(size.map(LruCache::sparse)),
            generation: AtomicU64::default(),
        }
    }

    pub fn cached_get_model_group_info<'call, D, R, P>(
        &'call self,
        lookup: &CatalogLookup<'_>,
        sources: &'call GroupSources<'call, D, R, P>,
        model_group: &'call str,
        keys: &'call mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<ModelGroupInfo>, GroupInfoError<R::Error>>> + Send + 'call
    where
        D: DeploymentInfo + Sync,
        R: ProviderResolver + Sync,
        P: SupportedParameterSource<Error = Infallible>,
    {
        let generation = self.generation.load(Ordering::SeqCst);
        let cached = {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries
                .as_mut()
                .and_then(|cache| cache.get(model_group).cloned())
        };
        // A miss starts the read now, so the catalog borrow never reaches the returned future.
        let pending = cached
            .is_none()
            .then(|| get_model_group_info(lookup, sources, model_group, keys));
        async move {
            let Some(started) = pending else {
                // Only a hit leaves pending empty, so the cached answer is there to return.
                return Ok(cached.flatten());
            };
            let answer = started.await?;
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            // A clear that landed during the read keeps that older answer out of the lru.
            if self.generation.load(Ordering::SeqCst) == generation
                && let Some(cache) = entries.as_mut()
            {
                drop(cache.put(model_group.to_owned(), answer.clone()));
            }
            drop(entries);
            Ok(answer)
        }
    }
}

impl CacheClear for GroupInfoCache {
    // _invalidate_model_group_info_cache, and the bump keeps an answer read before the clear out of the lru.
    fn clear(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cache) = entries.as_mut() {
            cache.clear();
        }
    }
}
