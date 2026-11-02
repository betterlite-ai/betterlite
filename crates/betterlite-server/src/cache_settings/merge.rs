use serde_json::{Map, Value};

use super::reuse::saved_secret_is_reusable;
use super::target::has_connection_target;
use super::{CACHE_SENSITIVE_FIELDS, REDACTED_VALUE};

// A save that leaves a credential untouched arrives marked or absent, so it is restored from the stored row when reusable and dropped otherwise; `url` is dropped outright once the caller switches to a discrete target.
#[must_use]
pub fn merge_over_saved(
    incoming: &Map<String, Value>,
    saved: &Map<String, Value>,
) -> Map<String, Value> {
    let switching_to_discrete_target =
        has_connection_target(incoming.get("host").unwrap_or(&Value::Null))
            || has_connection_target(incoming.get("redis_startup_nodes").unwrap_or(&Value::Null))
            || has_connection_target(incoming.get("sentinel_nodes").unwrap_or(&Value::Null));
    let reuse_saved_secret = saved_secret_is_reusable(incoming, saved);
    let mut merged = incoming.clone();
    for field in CACHE_SENSITIVE_FIELDS {
        if incoming
            .get(field)
            .is_some_and(|value| *value != REDACTED_VALUE)
        {
            continue;
        }
        if field == "url" && switching_to_discrete_target {
            merged.remove(field);
            continue;
        }
        if reuse_saved_secret && let Some(saved_value) = saved.get(field) {
            merged.insert(field.to_owned(), saved_value.clone());
            continue;
        }
        merged.remove(field);
    }
    merged
}
