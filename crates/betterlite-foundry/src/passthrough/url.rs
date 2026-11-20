use betterlite_azure::azure_urls::url_parts::first_query_values;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_openai::openai_base_config::Object;
use percent_encoding::percent_decode_str;

use crate::responses::path::{PathError, requoted_path};

use super::error::FoundryPassthroughError;

pub const API_VERSION: &str = "api-version";
const MODELS_SEGMENT: &str = "models";
const OPENAI_SEGMENT: &str = "openai";

// foundry_root: the base up to the first exact "models" segment, with the query dropped and the fragment kept
pub fn foundry_root(api_base: &str) -> Result<String, FoundryPassthroughError> {
    let url = HttpxUrl::parse(api_base).map_err(FoundryPassthroughError::InvalidApiBase)?;
    let segments = decoded_segments(&url);
    let end = segments
        .iter()
        .position(|segment| segment.as_str() == MODELS_SEGMENT)
        .unwrap_or(segments.len());
    rebuilt(url, segments.get(..end).unwrap_or_default())
}

#[must_use]
pub fn is_repeated_native_prefix(native_segments: &[String], overlap: usize) -> bool {
    overlap == native_segments.len()
        || native_segments
            .first()
            .is_some_and(|first| first.as_str() == OPENAI_SEGMENT)
}

// without_repeated_native_prefix: drop the longest root suffix the native endpoint repeats
pub fn without_repeated_native_prefix(
    root: &str,
    native_endpoint: &str,
) -> Result<String, FoundryPassthroughError> {
    let url = HttpxUrl::parse(root).map_err(FoundryPassthroughError::InvalidApiBase)?;
    let root_segments = decoded_segments(&url);
    let native_segments: Vec<String> = native_endpoint
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(caseless::default_case_fold_str)
        .collect();
    let overlap = overlap_length(&root_segments, &native_segments);
    let kept = root_segments.len().saturating_sub(overlap);
    rebuilt(url, root_segments.get(..kept).unwrap_or_default())
}

// relay_query_params: a caller api-version wins even when its value is null or empty
pub fn relay_query_params(
    request_query_params: Option<&Object>,
    deployment_api_version: Option<&str>,
    api_base: &str,
) -> Result<Option<Object>, FoundryPassthroughError> {
    if let Some(params) = request_query_params
        && !params.is_empty()
        && params.contains_key(API_VERSION)
    {
        return Ok(Some(params.clone()));
    }
    let url = HttpxUrl::parse(api_base).map_err(FoundryPassthroughError::InvalidApiBase)?;
    let base_version = first_query_values(url.query.as_deref())
        .into_iter()
        .find_map(|(name, value)| (name == API_VERSION).then_some(value));
    let version = deployment_api_version
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
        .or(base_version);
    let Some(version) = version else {
        return Ok(request_query_params.cloned());
    };
    let mut merged = request_query_params.cloned().unwrap_or_default();
    merged.insert(API_VERSION.to_owned(), serde_json::json!(version));
    Ok(Some(merged))
}

fn decoded_segments(url: &HttpxUrl) -> Vec<String> {
    percent_decode_str(&url.path)
        .decode_utf8_lossy()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn overlap_length(root_segments: &[String], native_segments: &[String]) -> usize {
    let longest = root_segments.len().min(native_segments.len());
    (1..=longest)
        .rev()
        .find(|&length| {
            let start = root_segments.len().saturating_sub(length);
            root_segments
                .get(start..)
                .zip(native_segments.get(..length))
                .is_some_and(|(root_suffix, native_prefix)| {
                    root_suffix
                        .iter()
                        .zip(native_prefix)
                        .all(|(root, native)| caseless::default_case_fold_str(root) == *native)
                        && is_repeated_native_prefix(native_segments, length)
                })
        })
        .unwrap_or(0)
}

// httpx's copy_with(path=...): the decoded path is parsed again, which requotes it and drops the query
fn rebuilt(mut url: HttpxUrl, segments: &[String]) -> Result<String, FoundryPassthroughError> {
    let path = format!("/{}", segments.join("/"));
    url.path = rewritten_path(&url, &path)?;
    url.query = None;
    Ok(url.href().trim_end_matches('/').to_owned())
}

fn rewritten_path(url: &HttpxUrl, path: &str) -> Result<String, FoundryPassthroughError> {
    requoted_path(url, path).map_err(|error| match error {
        PathError::Unrewritable => FoundryPassthroughError::UnwritableBasePath,
        PathError::InvalidUrl(source) => FoundryPassthroughError::InvalidApiBase(source),
    })
}
