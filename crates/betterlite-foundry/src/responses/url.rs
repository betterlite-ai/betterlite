use betterlite_http::user_url::error::InvalidUrl;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use percent_encoding::percent_decode_str;
use thiserror::Error;

use crate::credentials::model_info::ModelInfoAccessors;

use super::path::{PathError, requoted_path};

const RESPONSES_PATH: [&str; 3] = ["openai", "v1", "responses"];
const PROJECT_PREFIX: [&str; 2] = ["api", "projects"];

#[derive(Debug, Error)]
pub enum ResponsesUrlError {
    #[error(
        "api_base is required for the Azure AI Foundry Responses API. Set the api_base parameter or the AZURE_AI_API_BASE environment variable."
    )]
    MissingApiBase,
    #[error(transparent)]
    InvalidUrl(#[from] InvalidUrl),
    #[error(transparent)]
    Path(#[from] PathError),
}

pub fn complete_url(
    accessors: ModelInfoAccessors<'_>,
    api_base: Option<&str>,
) -> Result<String, ResponsesUrlError> {
    let resolved = accessors
        .api_base(api_base)
        .ok_or(ResponsesUrlError::MissingApiBase)?;
    responses_url(&resolved)
}

fn responses_url(api_base: &str) -> Result<String, ResponsesUrlError> {
    let original = HttpxUrl::parse(api_base)?;
    let decoded = percent_decode_str(&original.path).decode_utf8_lossy();
    let segments: Vec<&str> = decoded
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut parts: Vec<&str> = Vec::new();
    if segments.get(..2) == Some(&PROJECT_PREFIX[..]) {
        parts.extend(segments.get(..3).unwrap_or(&segments));
    }
    parts.extend(RESPONSES_PATH);
    let path = format!("/{}", parts.join("/"));
    let mut url = original;
    url.path = requoted_path(&url, &path)?;
    url.query = None;
    Ok(url.href())
}
