use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::capabilities::CachePointSupport;
use crate::bedrock_tools::error::BedrockToolError;
use crate::system_placement::cache_control::CacheTtl;

const CACHE_CONTROL: &str = "cache_control";

// Bedrock's only cache point type is default; a control LiteLLM does not recognize leaves the tool without one.
pub fn add_cache_point_tool_block(
    tool: &Object,
    support: CachePointSupport,
) -> Result<Option<Json>, BedrockToolError> {
    let control = match tool.get(CACHE_CONTROL) {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(control)) => control,
        Some(_) => {
            return Err(BedrockToolError::WrongType {
                field: CACHE_CONTROL,
            });
        }
    };
    if matches!(support, CachePointSupport::Unsupported) || !is_ephemeral(control.get("type")) {
        return Ok(None);
    }
    let mut point = Object::from_iter([("type".to_owned(), Json::Text("default".to_owned()))]);
    if matches!(support, CachePointSupport::ExtendedTtl)
        && let Some(ttl) = control.get("ttl").and_then(cache_ttl)
    {
        point.insert("ttl".to_owned(), Json::Text(ttl_text(ttl).to_owned()));
    }
    Ok(Some(Json::Object(Object::from_iter([(
        "cachePoint".to_owned(),
        Json::Object(point),
    )]))))
}

// An absent or null type is the ephemeral default; LiteLLM drops every other type without a word.
fn is_ephemeral(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => true,
        Some(Json::Text(text)) => text == "ephemeral",
        Some(_) => false,
    }
}

// TTLs LiteLLM does not know are left off the block rather than rejected, so the cache point still goes out.
fn cache_ttl(value: &Json) -> Option<CacheTtl> {
    match value.as_str() {
        Some("1h") => Some(CacheTtl::OneHour),
        Some("5m") => Some(CacheTtl::FiveMinutes),
        Some(_) | None => None,
    }
}

const fn ttl_text(ttl: CacheTtl) -> &'static str {
    match ttl {
        CacheTtl::OneHour => "1h",
        CacheTtl::FiveMinutes => "5m",
    }
}
