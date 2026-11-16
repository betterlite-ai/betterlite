use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::error::StreamItemsError;

// LiteLLM reads time.time() and uuid.uuid4(); both arrive here so an event can be compared exactly.
pub struct EventSources {
    pub clock: Arc<dyn Fn() -> Result<i64, StreamItemsError> + Send + Sync>,
    pub ids: Arc<dyn Fn() -> String + Send + Sync>,
}

impl EventSources {
    #[must_use]
    pub fn system() -> Self {
        Self {
            clock: Arc::new(|| {
                let elapsed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_error| StreamItemsError::ClockBeforeEpoch)?;
                i64::try_from(elapsed.as_secs()).map_err(|_error| StreamItemsError::ClockOutOfRange)
            }),
            ids: Arc::new(|| Uuid::new_v4().to_string()),
        }
    }
}
