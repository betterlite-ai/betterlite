use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::event::ProxyServerRequest;

const HEADER_PREFIX: &str = "helicone_";

// The prefix match is exact and case sensitive, and a header beats a metadata key of the same name.
#[must_use]
pub fn merge(
    metadata: &IndexMap<String, Json>,
    proxy_server_request: Option<&ProxyServerRequest>,
) -> IndexMap<String, Json> {
    let mut merged = metadata.clone();
    if let Some(request) = proxy_server_request {
        for (name, value) in &request.headers {
            if name.starts_with(HEADER_PREFIX) {
                merged.insert(name.clone(), Json::Text(value.clone()));
            }
        }
    }
    merged
}

mod tests;
