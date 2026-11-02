use serde_json::{Map, Value};

use super::URL_OVERRIDDEN_FIELDS;
use super::target::has_connection_target;

// redis.Redis.from_url ignores the discrete fields, so a full url wins; cluster mode is exempt since it has no url form.
#[must_use]
pub fn resolve_cache_url_precedence(settings: &Map<String, Value>) -> Map<String, Value> {
    let has_url =
        matches!(settings.get("url"), Some(Value::String(text)) if !text.trim().is_empty());
    let cluster_mode = settings
        .get("redis_startup_nodes")
        .is_some_and(has_connection_target);
    if !has_url || cluster_mode {
        return settings.clone();
    }
    settings
        .iter()
        .filter(|(key, _)| !URL_OVERRIDDEN_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
