use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::HandlerError;
use betterlite_cache::handler::kwargs::request_control;

// LiteLLM's outer condition: false means _async_get_cache returned None and the caller runs the call itself
pub fn async_cache_enabled(kwargs: &KeyObject, cache_present: bool) -> Result<bool, HandlerError> {
    let caching = match kwargs.get("caching") {
        None | Some(KeyValue::Null) => None,
        Some(KeyValue::Bool(flag)) => Some(*flag),
        Some(_) => {
            return Err(HandlerError::FieldType {
                field: "caching",
                expected: "a boolean",
            });
        }
    };
    // absent or null takes the cache's presence, an explicit value is used as given
    let enabled = caching.unwrap_or(cache_present);
    if !enabled {
        return Ok(false);
    }
    // LiteLLM folds cache.no-cache into the same condition, so a request that asked for no cached responses also gets None
    Ok(request_control(kwargs)?.no_cache != Some(true))
}
