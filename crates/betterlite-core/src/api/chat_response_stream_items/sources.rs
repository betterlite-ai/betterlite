use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

// LiteLLM reads time.time() and uuid.uuid4(); both arrive here so an event can be compared exactly.
pub struct EventSources {
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ids: Arc<dyn Fn() -> String + Send + Sync>,
}

impl EventSources {
    #[must_use]
    pub fn new(
        clock: Arc<dyn Fn() -> i64 + Send + Sync>,
        ids: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> Self {
        Self { clock, ids }
    }

    #[must_use]
    pub fn system() -> Self {
        Self::new(
            Arc::new(|| {
                let elapsed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            }),
            Arc::new(|| Uuid::new_v4().to_string()),
        )
    }

    pub(super) fn now(&self) -> i64 {
        (self.clock)()
    }

    pub(super) fn next_id(&self) -> String {
        (self.ids)()
    }
}
