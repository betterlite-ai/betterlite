use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_context::dynamic_logging_cache::{Clock, lookup_marked, store_marked};
use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache};
use betterlite_http::skill_archive::SkillArchive;

// LiteLLM's InMemoryCache(max_size_in_memory=128, default_ttl=3600) over the built archives
const MAX_CACHED_ARCHIVES: i64 = 128;
const ARCHIVE_CACHE_TTL_SECONDS: f64 = 3_600.0;
// the TTL cache only holds null markers here, so its per-item limit fits one; an archive's own 512 KiB bound is checked by the route
const MARKER_ITEM_KIB: i64 = 1_024;

struct Entries {
    markers: LocalTtlCache,
    archives: BTreeMap<String, SkillArchive>,
}

pub(crate) struct ArchiveCache {
    entries: Mutex<Entries>,
}

impl ArchiveCache {
    pub(crate) fn new(clock: Clock) -> Result<Self, CacheError> {
        let markers = LocalTtlCache::with_clock(
            Some(MAX_CACHED_ARCHIVES),
            Some(ARCHIVE_CACHE_TTL_SECONDS),
            Some(MARKER_ITEM_KIB),
            MARKER_ITEM_KIB,
            clock,
        )?;
        Ok(Self {
            entries: Mutex::new(Entries {
                markers,
                archives: BTreeMap::new(),
            }),
        })
    }

    pub(crate) fn lookup(&self, key: &str) -> Option<SkillArchive> {
        let mut guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entries = &mut *guard;
        let found = lookup_marked(&mut entries.markers, &mut entries.archives, key);
        drop(guard);
        match found {
            Ok((cached, _expired)) => cached,
            Err(error) => {
                tracing::debug!(error = %error, "agent skills archive cache lookup failed");
                None
            }
        }
    }

    pub(crate) fn store(&self, key: &str, archive: &SkillArchive) {
        let mut guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entries = &mut *guard;
        let stored = store_marked(
            &mut entries.markers,
            &mut entries.archives,
            key.to_owned(),
            archive.clone(),
            None,
        );
        drop(guard);
        if let Err(error) = stored {
            tracing::debug!(error = %error, "agent skills archive cache store failed");
        }
    }
}
