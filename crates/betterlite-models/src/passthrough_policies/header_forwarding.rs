use indexmap::IndexMap;

use super::passthrough_headers::forward_passthrough_headers;

pub const REMOVED_REQUEST_HEADERS: [&str; 3] = ["content-length", "host", "accept-encoding"];

#[must_use]
pub fn forward_headers_from_request(
    request_headers: &IndexMap<String, String>,
    headers: IndexMap<String, String>,
    forward_headers: bool,
) -> IndexMap<String, String> {
    let mut forwarded = request_headers.clone();
    let mut merged: IndexMap<String, String> = IndexMap::new();
    if forward_headers {
        for name in REMOVED_REQUEST_HEADERS {
            forwarded.shift_remove(name);
        }
        for name in headers.keys() {
            let lowered = name.to_lowercase();
            forwarded.retain(|request_name, _value| request_name.to_lowercase() != lowered);
        }
        for (name, value) in &forwarded {
            merged.insert(name.clone(), value.clone());
        }
    }
    for (name, value) in headers {
        merged.insert(name, value);
    }
    forward_passthrough_headers(&forwarded, &mut merged);
    merged
}

mod tests;
