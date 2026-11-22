mod tests;

use betterlite_db::mcp_stored_contracts::JsonValue;

// RFC 7591 redirect_uris must be a non-empty array of non-empty strings; every other shape falls back to the gateway callback.
#[must_use]
pub fn client_supplied_redirect_uris(value: Option<&JsonValue>) -> Option<Vec<String>> {
    let JsonValue::Array(entries) = value? else {
        return None;
    };
    if entries.is_empty() {
        return None;
    }
    let mut uris = Vec::with_capacity(entries.len());
    for entry in entries {
        let JsonValue::String(uri) = entry else {
            return None;
        };
        if uri.is_empty() {
            return None;
        }
        uris.push(uri.clone());
    }
    Some(uris)
}
