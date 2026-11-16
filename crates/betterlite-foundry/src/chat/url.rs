use betterlite_azure::azure_urls::url_parts::{encoded_query, first_query_values};
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_openai::openai_base_config::overlapping_api_base_path;

use super::url_error::UrlError;

const FOUNDRY_HOST: &str = "services.ai.azure.com";
const FOUNDRY_ROUTE: &str = "/models/chat/completions";
const STUDIO_ROUTE: &str = "/chat/completions";
const API_VERSION_PARAM: &str = "api-version";

// The raw string decides the route, so a base whose host is not a Foundry host can still take the models path
pub fn get_complete_url(
    api_base: Option<&str>,
    api_version: Option<&str>,
) -> Result<String, UrlError> {
    let Some(base) = api_base else {
        return Err(UrlError::MissingApiBase);
    };
    let original = HttpxUrl::parse(base).map_err(UrlError::InvalidUrl)?;
    let mut query = first_query_values(original.query.as_deref());
    if !query.iter().any(|(name, _)| name == API_VERSION_PARAM)
        && let Some(version) = api_version.filter(|text| !text.is_empty())
    {
        query.push((String::from(API_VERSION_PARAM), version.to_owned()));
    }
    let ending = if base.contains(FOUNDRY_HOST) {
        FOUNDRY_ROUTE
    } else {
        STUDIO_ROUTE
    };
    let merged = overlapping_api_base_path(base, ending).map_err(UrlError::Compose)?;
    let mut url = HttpxUrl::parse(&merged).map_err(UrlError::InvalidUrl)?;
    url.query = encoded_query(&query);
    Ok(url.href())
}
