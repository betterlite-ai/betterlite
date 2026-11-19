use uuid::Uuid;

use super::ports::ConfigEndpointsSource;
use super::record::PassThroughGenericEndpoint;

// LiteLLM registers config endpoints once at startup and keeps the id it assigned; here the id is generated per read
#[must_use]
pub fn config_endpoints(source: &dyn ConfigEndpointsSource) -> Vec<PassThroughGenericEndpoint> {
    let Some(entries) = source.configured_endpoints() else {
        return Vec::new();
    };
    let mut endpoints = Vec::with_capacity(entries.len());
    for entry in entries {
        match PassThroughGenericEndpoint::from_json(entry, true) {
            Ok(mut endpoint) => {
                if endpoint.id.is_none() {
                    endpoint.id = Some(Uuid::new_v4().to_string());
                }
                endpoints.push(endpoint);
            }
            Err(error) => {
                tracing::warn!(%error, "skipping a malformed pass-through endpoint from the config file");
            }
        }
    }
    endpoints
}
