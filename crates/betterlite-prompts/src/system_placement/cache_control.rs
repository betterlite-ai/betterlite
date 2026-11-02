use crate::arguments::json::{Json, Object};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheTtl {
    OneHour,
    FiveMinutes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheControl {
    pub ttl: Option<CacheTtl>,
}

// The client's cache_control rebuilt in the only shape Anthropic accepts: an unrecognized ttl falls back to bare ephemeral rather than dropping the cache_control entirely.
#[must_use]
pub fn cache_control(fields: &Object) -> Option<CacheControl> {
    let value = fields.get("cache_control")?.as_object()?;
    if value.get("type").and_then(Json::as_str) != Some("ephemeral") {
        return None;
    }
    let ttl = match value.get("ttl").and_then(Json::as_str) {
        Some("1h") => Some(CacheTtl::OneHour),
        Some("5m") => Some(CacheTtl::FiveMinutes),
        Some(_) | None => None,
    };
    Some(CacheControl { ttl })
}

#[must_use]
pub fn cache_control_block(block: &CacheControl) -> Json {
    let mut fields = Object::from_iter([("type".to_owned(), Json::Text("ephemeral".to_owned()))]);
    if let Some(ttl) = block.ttl {
        let ttl_text = match ttl {
            CacheTtl::OneHour => "1h",
            CacheTtl::FiveMinutes => "5m",
        };
        fields.insert("ttl".to_owned(), Json::Text(ttl_text.to_owned()));
    }
    Json::Object(fields)
}

mod tests;
