use betterlite_openai::openai_base_config::overlapping_api_base_path;
use url::Url;
use url::form_urlencoded;

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
    let original = Url::parse(base).map_err(UrlError::InvalidUrl)?;
    let mut query = collapsed_query_params(original.query());
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
    let mut url = Url::parse(&merged).map_err(UrlError::InvalidUrl)?;
    url.set_query(encoded_query(&query).as_deref());
    Ok(url.to_string())
}

fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

// dict() reads httpx's QueryParams through its mapping protocol, so a repeated name keeps its first value
fn collapsed_query_params(query: Option<&str>) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for (name, value) in query_pairs(query) {
        if !kept.iter().any(|(kept_name, _)| *kept_name == name) {
            kept.push((name, value));
        }
    }
    kept
}

fn encoded_query(pairs: &[(String, String)]) -> Option<String> {
    if pairs.is_empty() {
        return None;
    }
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(name, value);
    }
    // Python's urlencode encodes * and keeps ~, the reverse of form_urlencoded
    Some(serializer.finish().replace('*', "%2A").replace("%7E", "~"))
}
