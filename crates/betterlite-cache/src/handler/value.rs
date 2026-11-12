use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;

// the dual cache stores betterlite_models::Json while the envelope stores CacheValue
pub(crate) fn to_cache_value(json: &Json) -> Option<CacheValue> {
    match serde_json::to_value(json) {
        Ok(wire) => serde_json::from_value(wire).ok(),
        Err(_failure) => None,
    }
}

pub(crate) fn to_json(value: &CacheValue) -> Option<Json> {
    match serde_json::to_value(value) {
        Ok(wire) => serde_json::from_value(wire).ok(),
        Err(_failure) => None,
    }
}
