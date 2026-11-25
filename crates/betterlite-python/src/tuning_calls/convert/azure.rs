use http::header::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::super::error::WireError;
use super::insert_header;
use super::requests::JSON_CONTENT;

// LiteLLM's Azure client asks for this version when neither the call nor the environment names one
const AZURE_DEFAULT_API_VERSION: &str = "2025-02-01-preview";
// LiteLLM treats exactly these names as the Azure v1 surface, matched case-sensitively
const V1_API_VERSIONS: [&str; 3] = ["preview", "latest", "v1"];

pub(super) fn azure_version(version: Option<&str>) -> &str {
    version
        .filter(|value| !value.is_empty())
        .unwrap_or(AZURE_DEFAULT_API_VERSION)
}

// the v1 surface is a plain OpenAI client under {api_base}/openai/v1, and everything else is AzureOpenAI
pub(super) fn azure_jobs_url(
    api_base: &str,
    version: &str,
    suffix: &str,
) -> Result<Url, WireError> {
    let base = api_base.trim_end_matches('/');
    let target = if V1_API_VERSIONS.contains(&version) {
        format!("{api_base}/openai/v1/fine_tuning/jobs{suffix}")
    } else if api_base.contains("/openai/deployments") {
        format!("{base}/fine_tuning/jobs{suffix}")
    } else {
        format!("{base}/openai/fine_tuning/jobs{suffix}")
    };
    Url::parse(&target).map_err(|source| WireError::Url { source })
}

// the v1 surface carries no api-version in the query, and every other version names it
pub(super) fn azure_query(version: &str) -> Vec<(String, String)> {
    if V1_API_VERSIONS.contains(&version) {
        return Vec::new();
    }
    vec![("api-version".to_owned(), version.to_owned())]
}

// the v1 surface authenticates with a bearer token, and AzureOpenAI with the api-key header
pub(super) fn azure_headers(key: &SecretString, version: &str) -> Result<HeaderMap, WireError> {
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "accept", JSON_CONTENT)?;
    if V1_API_VERSIONS.contains(&version) {
        insert_header(
            &mut headers,
            "authorization",
            &format!("Bearer {}", key.expose_secret()),
        )?;
    } else {
        insert_header(&mut headers, "api-key", key.expose_secret())?;
    }
    Ok(headers)
}
