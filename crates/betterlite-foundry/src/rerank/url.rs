use betterlite_azure::azure_urls::url_parts::{encoded_query, query_pairs};
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_openai::openai_base_config::overlapping_api_base_path;
use percent_encoding::percent_decode_str;

use super::error::FoundryRerankError;

const V1_RERANK: &str = "/v1/rerank";
const V2_RERANK: &str = "/v2/rerank";
const V1_VERSION: &str = "/v1";
const V2_VERSION: &str = "/v2";
const COHERE_V2_VERSION: &str = "/providers/cohere/v2";
const RERANK: &str = "/rerank";

// LiteLLM has no default Foundry base: the caller or AZURE_AI_API_BASE supplies an absolute URL
pub fn complete_url(api_base: Option<&str>) -> Result<String, FoundryRerankError> {
    let Some(base) = api_base else {
        return Err(FoundryRerankError::MissingApiBase);
    };
    let original = HttpxUrl::parse(base).map_err(FoundryRerankError::InvalidUrl)?;
    if original.scheme.is_empty() || original.host.is_empty() {
        return Err(FoundryRerankError::RelativeApiBase {
            rendered: quoted(base),
        });
    }
    let path = decoded_path(&original)?;
    let trimmed = path.trim_end_matches('/');
    if trimmed.ends_with(V1_RERANK) || trimmed.ends_with(V2_RERANK) {
        let mut kept = original;
        kept.path = trimmed.to_owned();
        return Ok(kept.href());
    }
    if trimmed.ends_with(V1_VERSION)
        || trimmed.ends_with(V2_VERSION)
        || trimmed.ends_with(COHERE_V2_VERSION)
    {
        let mut versioned = original;
        versioned.path = trimmed.to_owned();
        return append_path(&versioned.href(), RERANK);
    }
    append_path(base, V1_RERANK)
}

// httpx reads the path decoded and writes it back through its own quoting, so an escape that re-quotes differently changes the URL
fn decoded_path(url: &HttpxUrl) -> Result<String, FoundryRerankError> {
    let decoded = percent_decode_str(&url.path).decode_utf8_lossy();
    if decoded.contains(['?', '#']) {
        return Err(FoundryRerankError::UnwritablePath);
    }
    let mut authority = url.clone();
    authority.path = String::new();
    authority.query = None;
    authority.fragment = None;
    let composed = HttpxUrl::parse(&format!("{}{decoded}", authority.href()))
        .map_err(FoundryRerankError::InvalidUrl)?;
    Ok(composed.path)
}

// B668's shared merge appends the route once, and the query comes back through the same re-encoding httpx applies
fn append_path(api_base: &str, ending: &str) -> Result<String, FoundryRerankError> {
    let original = HttpxUrl::parse(api_base).map_err(FoundryRerankError::InvalidUrl)?;
    let mut canonical = original.clone();
    canonical.path = decoded_path(&original)?;
    let merged = overlapping_api_base_path(&canonical.href(), ending)
        .map_err(FoundryRerankError::Compose)?;
    let mut url = HttpxUrl::parse(&merged).map_err(FoundryRerankError::InvalidUrl)?;
    url.query = encoded_query(&query_pairs(original.query.as_deref()));
    Ok(url.href())
}

// The diagnostic carries the base the way LiteLLM's repr does; only the text of an error message depends on it
fn quoted(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push(quote);
    for letter in text.chars() {
        match letter {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            _ if letter == quote => {
                rendered.push('\\');
                rendered.push(letter);
            }
            _ => rendered.push(letter),
        }
    }
    rendered.push(quote);
    rendered
}
