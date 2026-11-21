use betterlite_core::agents::json::AgentJson;
use betterlite_logging::private_json::safe_json::MAX_DEPTH_MARKER;
use betterlite_logging::private_json::safe_json::safe_dumps;

// A JSON value always encodes, so the fallback only keeps the signature total
#[must_use]
pub fn safe_json_text(value: &AgentJson, max_depth: usize) -> String {
    let Ok(encoded) = safe_dumps(value, max_depth, None) else {
        return String::from(MAX_DEPTH_MARKER);
    };
    encoded
}

mod tests;
