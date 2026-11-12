use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::url_path::encode_segment;
use serde::Deserialize;

use super::API_VERSION;
use super::credentials::{StudioParams, resolve_api_base, resolve_api_key};
use super::error::StudioInteractionError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Get,
    Delete,
    Cancel,
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
) -> Result<String, StudioInteractionError> {
    if resolve_api_key(params, settings).is_none() {
        return Err(StudioInteractionError::MissingApiKey);
    }
    let encoded = encode_segment(interaction_id, "interaction_id")
        .map_err(StudioInteractionError::InteractionId)?;
    let base = resolve_api_base(api_base, settings);
    let suffix = match action {
        Lifecycle::Get | Lifecycle::Delete => "",
        Lifecycle::Cancel => ":cancel",
    };
    Ok(format!(
        "{base}/{API_VERSION}/interactions/{encoded}{suffix}"
    ))
}
