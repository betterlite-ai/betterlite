use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;

pub const CACHE_TTL_SECONDS: f64 = 900.0;

// The source stamps and prunes with time.time(), so the wall clock is a value tests replace.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub struct CachedContent {
    pub content: IndexMap<String, String>,
    pub created_at: f64,
}

pub struct CompressionCache {
    entries: Mutex<IndexMap<String, CachedContent>>,
    clock: Clock,
}

impl CompressionCache {
    #[must_use]
    pub fn with_clock(clock: Clock) -> Self {
        Self {
            entries: Mutex::new(IndexMap::new()),
            clock,
        }
    }

    pub fn store(&self, call_id: String, content: IndexMap<String, String>) {
        let created_at = (self.clock)();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.insert(
            call_id,
            CachedContent {
                content,
                created_at,
            },
        );
    }

    pub fn prune(&self) {
        let now = (self.clock)();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        // float_arithmetic is denied; a fused multiply-add computes now - created_at
        entries.retain(|_, entry| entry.created_at.mul_add(-1.0_f64, now) <= CACHE_TTL_SECONDS);
    }

    #[must_use]
    pub fn entry(&self, call_id: &str) -> Option<CachedContent> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(call_id)
            .cloned()
    }
}

impl Default for CompressionCache {
    fn default() -> Self {
        Self::with_clock(Arc::new(system_now))
    }
}

#[must_use]
pub fn system_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
