use serde::Deserialize;

use super::error::CacheLookupError;

pub const CACHE_PAGE_LIMIT: usize = 4 * 1024 * 1024;

// LiteLLM's CachedContentListAllResponseBody: both fields are optional, and a null takes the absent default
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct CachedPage {
    #[serde(default, rename = "cachedContents")]
    pub cached_contents: Option<Vec<CachedContent>>,
    #[serde(default, rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

// The list loop reads the name it returns and the displayName it matches; the rest of LiteLLM's CachedContent is not read here
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct CachedContent {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

pub fn decode_cached_page(bytes: &[u8]) -> Result<CachedPage, CacheLookupError> {
    if bytes.len() > CACHE_PAGE_LIMIT {
        return Err(CacheLookupError::PageTooLarge {
            limit: CACHE_PAGE_LIMIT,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| CacheLookupError::Decode { source })
}
