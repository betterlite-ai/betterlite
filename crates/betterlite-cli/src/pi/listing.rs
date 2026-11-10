use std::collections::HashSet;

use secrecy::SecretString;
use serde::Deserialize;

use super::{FETCH_TIMEOUT, ListedModel, ListingViolation, PiSyncError, ProxyGet};

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ListedModel>,
}

pub fn fetch_model_listing(
    base_url: &str,
    api_key: &SecretString,
    get: &dyn ProxyGet,
    extra_headers: &[(&str, &str)],
) -> Result<Vec<ListedModel>, PiSyncError> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let sent_headers = super::request_headers(api_key, extra_headers);
    let response = get
        .get(&url, &sent_headers, FETCH_TIMEOUT)
        .map_err(|source| PiSyncError::Unreachable { source })?;
    if response.status != 200 {
        return Err(PiSyncError::ListingStatus {
            status: response.status,
        });
    }
    let listing = serde_json::from_slice::<ModelList>(&response.body)
        .map_err(|source| PiSyncError::ListingJson { source })?;
    validate_mappings(&listing.data)?;
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for model in listing.data {
        if seen.insert(model.clone()) {
            models.push(model);
        }
    }
    if models.is_empty() {
        return Err(PiSyncError::NoModels);
    }
    Ok(models)
}

pub fn fetch_model_ids(
    base_url: &str,
    api_key: &SecretString,
    get: &dyn ProxyGet,
    extra_headers: &[(&str, &str)],
) -> Result<Vec<String>, PiSyncError> {
    let models = fetch_model_listing(base_url, api_key, get, extra_headers)?;
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for model in models {
        if seen.insert(model.id.clone()) {
            ids.push(model.id);
        }
    }
    Ok(ids)
}

fn validate_mappings(models: &[ListedModel]) -> Result<(), PiSyncError> {
    for model in models {
        if model.id.is_empty() {
            return Err(PiSyncError::ListingInvalid {
                violation: ListingViolation::EmptyId,
            });
        }
        if model.source_model.as_deref() == Some("") {
            return Err(PiSyncError::ListingInvalid {
                violation: ListingViolation::EmptySourceModel,
            });
        }
    }
    let ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<HashSet<&str>>();
    let mappings = models
        .iter()
        .map(|model| {
            (
                model.id.as_str(),
                model.source_model.as_deref().unwrap_or(&model.id),
            )
        })
        .collect::<HashSet<(&str, &str)>>();
    if ids.len() != mappings.len() {
        return Err(PiSyncError::ListingInvalid {
            violation: ListingViolation::DuplicateMappings,
        });
    }
    Ok(())
}
