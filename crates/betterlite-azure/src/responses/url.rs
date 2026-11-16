use betterlite_config::sections::azure_responses::Settings as ResponsesSettings;
use betterlite_http::user_url::httpx_url::HttpxUrl;

use super::error::ResponsesError;
use crate::azure_urls::{AzureUrlError, get_base_azure_url};

pub const ROUTE: &str = "/openai/responses";
pub const WEBSOCKET_ROUTE: &str = "/openai/v1/responses";
// LiteLLM sends the model in the response.create body, so it never appears in the websocket URL
pub const MODEL_IN_WEBSOCKET_URL: bool = false;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = true;

// The version only fills the query when the caller and the api_base both leave it open
pub fn complete_url(
    api_base: Option<&str>,
    api_version: Option<&str>,
    settings: &ResponsesSettings,
) -> Result<String, AzureUrlError> {
    get_base_azure_url(
        api_base,
        api_version,
        Some(&settings.default_responses_api_version),
        ROUTE,
    )
}

// httpx's copy_with(scheme=..., path=..., query=None): the path is re-parsed, the fragment is kept
pub fn websocket_url(api_base: Option<&str>) -> Result<String, ResponsesError> {
    let Some(base) = api_base else {
        return Err(ResponsesError::MissingWebsocketApiBase);
    };
    let parsed = HttpxUrl::parse(base)?;
    // one suffix comes off, in LiteLLM's order, so a base already at the websocket route is not appended twice
    let trimmed = parsed.path.trim_end_matches('/');
    let path = trimmed
        .strip_suffix(WEBSOCKET_ROUTE)
        .or_else(|| trimmed.strip_suffix(ROUTE))
        .unwrap_or(trimmed);
    let mut authority = parsed.clone();
    authority.path = String::new();
    authority.query = None;
    authority.fragment = None;
    let composed = format!("{}{path}{WEBSOCKET_ROUTE}", authority.href());
    let mut websocket = HttpxUrl::parse(&composed)?;
    websocket.scheme = if parsed.scheme == "https" {
        String::from("wss")
    } else {
        String::from("ws")
    };
    websocket.query = None;
    websocket.fragment = parsed.fragment;
    Ok(websocket.href())
}
