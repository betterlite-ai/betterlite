use indexmap::IndexMap;

use super::protected_headers::is_protected;

pub const PASSTHROUGH_HEADER_PREFIX: &str = "x-pass-";

pub fn passthrough_header_name(name: &str) -> Option<String> {
    if !name.to_lowercase().starts_with(PASSTHROUGH_HEADER_PREFIX) {
        return None;
    }
    name.get(PASSTHROUGH_HEADER_PREFIX.len()..)
        .map(str::to_lowercase)
}

pub fn forward_passthrough_headers(
    request_headers: &IndexMap<String, String>,
    headers: &mut IndexMap<String, String>,
) {
    for (name, value) in request_headers {
        let Some(actual) = passthrough_header_name(name) else {
            continue;
        };
        if is_protected(&actual) {
            tracing::debug!(header = %name, "x-pass- header names a protected header; skipped");
            continue;
        }
        headers.insert(actual, value.clone());
    }
}

mod tests;
