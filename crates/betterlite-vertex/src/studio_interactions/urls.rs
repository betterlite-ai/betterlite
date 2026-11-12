use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::url_path::encode_segment;
use indexmap::IndexMap;

use super::API_VERSION;
use super::credentials::{StudioParams, resolve_api_base, resolve_api_key};
use super::error::StudioInteractionError;
use betterlite_models::decisions::Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Get,
    Delete,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleRequest {
    pub url: String,
    pub body: IndexMap<String, Json>,
}

pub fn create_url(
    api_base: Option<&str>,
    params: &StudioParams,
    stream: bool,
    settings: &Settings,
) -> Result<String, StudioInteractionError> {
    if resolve_api_key(params, settings).is_none() {
        return Err(StudioInteractionError::MissingApiKey);
    }
    let base = resolve_api_base(api_base, settings);
    let query = if stream { "?alt=sse" } else { "" };
    Ok(format!("{base}/{API_VERSION}/interactions{query}"))
}

pub fn lifecycle_request(
    action: Lifecycle,
    interaction_id: &str,
    api_base: Option<&str>,
    params: &StudioParams,
    settings: &Settings,
) -> Result<LifecycleRequest, StudioInteractionError> {
    if resolve_api_key(params, settings).is_none() {
        return Err(StudioInteractionError::MissingApiKey);
    }
    let encoded = encode_segment(interaction_id, "interaction_id")
        .map_err(|source| StudioInteractionError::InteractionId { source })?;
    let base = resolve_api_base(api_base, settings);
    let suffix = match action {
        Lifecycle::Get | Lifecycle::Delete => "",
        Lifecycle::Cancel => ":cancel",
    };
    Ok(LifecycleRequest {
        url: format!("{base}/{API_VERSION}/interactions/{encoded}{suffix}"),
        body: IndexMap::new(),
    })
}
