use chrono::{DateTime, Utc};
use indexmap::IndexMap;

// A window's deployment counts in the order log_success_event inserted them, which is the order a tie keeps.
pub type Counts = IndexMap<String, i64>;

// The one failure a window store reports: LiteLLM's cache swallows it and its caller reads a cold window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WindowError {
    #[error("the usage window store is unavailable")]
    Unavailable,
}

// The cache behind log_success_event's read-modify-write and the two reads get_available_deployments makes.
pub trait UsageWindows {
    fn read(&self, key: &str) -> Result<Option<Counts>, WindowError>;

    fn write(&self, key: &str, counts: &Counts, ttl_seconds: i64) -> Result<(), WindowError>;
}

// log_success_event's f"{model_group}:tpm:{datetime.now().strftime('%H-%M')}" pair. The label reads the UTC
// minute, so every instance buckets a request into the same window whatever its host clock's zone is.
#[must_use]
pub fn window_keys(group: &str, now: DateTime<Utc>) -> (String, String) {
    let minute = now.format("%H-%M");
    (
        format!("{group}:tpm:{minute}"),
        format!("{group}:rpm:{minute}"),
    )
}
