use indexmap::IndexMap;
use url::{Url, form_urlencoded};

use super::error::RealtimeError;

// LiteLLM's RealtimeQueryParams values: every key is optional and intent may be null
pub type QueryParams = IndexMap<String, Option<String>>;

pub fn construct_url(api_base: &str, query_params: &QueryParams) -> Result<String, RealtimeError> {
    // the source swaps the scheme textually before parsing, so a https:// inside the query swaps too
    let swapped = api_base
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let mut url =
        Url::parse(&swapped).map_err(|source| RealtimeError::InvalidApiBase { source })?;
    url.set_path("/v1/realtime");
    if !query_params.is_empty() {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        serializer.extend_pairs(
            query_params
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_deref().unwrap_or_default())),
        );
        // httpx encodes a space as + and / as %2F, but keeps * and ~ as the query serializer does
        let encoded = serializer.finish().replace('*', "%2A").replace("%7E", "~");
        url.set_query(Some(&encoded));
    }
    Ok(url.to_string())
}
