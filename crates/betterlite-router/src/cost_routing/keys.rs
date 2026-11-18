use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};

// The pinned handler writes datetime.now().strftime("%Y-%m-%d-%H-%M"); betterlite reads the same minute in UTC.
#[must_use]
pub fn minute_label(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let moment = i64::try_from(seconds)
        .ok()
        .and_then(|epoch| DateTime::<Utc>::from_timestamp(epoch, 0))
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    moment.format("%Y-%m-%d-%H-%M").to_string()
}

// One cost_map:{model_group} entry of the pinned handler, split into the two counters a store can increment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowKeys {
    pub tokens: String,
    pub requests: String,
}

#[must_use]
pub fn window_keys(model_group: &str, deployment_id: &str, minute: &str) -> WindowKeys {
    WindowKeys {
        tokens: format!("cost_map:{model_group}:{deployment_id}:{minute}:tpm"),
        requests: format!("cost_map:{model_group}:{deployment_id}:{minute}:rpm"),
    }
}
