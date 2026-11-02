use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use regex::Regex;

use super::endpoint_metadata::label_and_path;
use super::error::ProvidersError;
use super::types::{
    EndpointProvider, ProvidersFile, SupportedEndpoint, SupportedEndpointsResponse,
};

const MAX_ENDPOINTS_BYTES: usize = 1024 * 1024;
pub(super) const BACKUP_ASSET: &[u8] =
    include_bytes!("fixtures/provider_endpoints_support_backup.json");

#[must_use]
pub fn clean_display_name(raw: &str) -> String {
    Regex::new(r"\s*\(`[^`]+`\)\s*$").ok().map_or_else(
        || raw.trim().to_owned(),
        |pattern| pattern.replace(raw, "").trim().to_owned(),
    )
}

pub(super) fn build_endpoints(raw: &ProvidersFile) -> Vec<SupportedEndpoint> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for entry in raw.providers.values() {
        for key in entry.endpoints.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    keys.into_iter()
        .map(|key| {
            let (label, endpoint) = label_and_path(&key);
            let providers = raw
                .providers
                .iter()
                .filter(|(_, entry)| entry.endpoints.get(&key).copied().unwrap_or(false))
                .map(|(slug, entry)| EndpointProvider {
                    slug: slug.clone(),
                    display_name: clean_display_name(entry.display_name.as_deref().unwrap_or(slug)),
                })
                .collect();
            SupportedEndpoint {
                key,
                label,
                endpoint,
                providers,
            }
        })
        .collect()
}

pub fn build_endpoints_from_json(bytes: &[u8]) -> Result<Vec<SupportedEndpoint>, ProvidersError> {
    if bytes.len() > MAX_ENDPOINTS_BYTES {
        return Err(ProvidersError::EndpointsTooLarge {
            size: bytes.len(),
            limit: MAX_ENDPOINTS_BYTES,
        });
    }
    let raw: ProvidersFile =
        serde_json::from_slice(bytes).map_err(ProvidersError::EndpointsJson)?;
    Ok(build_endpoints(&raw))
}

static CACHED_ENDPOINTS: Mutex<Option<Arc<SupportedEndpointsResponse>>> = Mutex::new(None);

pub fn supported_endpoints() -> Result<Arc<SupportedEndpointsResponse>, ProvidersError> {
    let mut cached = CACHED_ENDPOINTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(endpoints) = cached.as_ref() {
        return Ok(Arc::clone(endpoints));
    }
    let endpoints = Arc::new(SupportedEndpointsResponse {
        endpoints: build_endpoints_from_json(BACKUP_ASSET)?,
    });
    *cached = Some(Arc::clone(&endpoints));
    drop(cached);
    Ok(endpoints)
}
