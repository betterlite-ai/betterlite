use crate::arguments::json::Json;
use crate::system_placement::cache_control::{CacheControl, CacheTtl, cache_control_block};

// LiteLLM honors only these two ttl spellings and leaves every other value on the bare 5-minute ephemeral cache.
#[must_use]
pub fn default_control(ttl: Option<&str>) -> Json {
    let policy = match ttl {
        Some("5m") => Some(CacheTtl::FiveMinutes),
        Some("1h") => Some(CacheTtl::OneHour),
        Some(_) | None => None,
    };
    cache_control_block(&CacheControl { ttl: policy })
}
