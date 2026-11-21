use betterlite_prompts::arguments::json::Object as Fields;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use super::error::PerplexitySearchError;
use crate::openai_base_config::Object;
use crate::search::records::{SearchObject, SearchResponse, SearchResult};

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

#[serde_as]
#[derive(Deserialize)]
struct WireReply {
    #[serde_as(as = "DefaultOnNull")]
    #[serde(default)]
    results: Vec<WireResult>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Fields,
}

// LiteLLM reads these five fields of a result and drops the rest
#[serde_as]
#[derive(Deserialize)]
struct WireResult {
    #[serde_as(as = "DefaultOnNull")]
    #[serde(default)]
    title: String,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(default)]
    url: String,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    last_updated: Option<String>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Fields,
}

pub fn decode_search_reply(bytes: &[u8]) -> Result<SearchResponse, PerplexitySearchError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(PerplexitySearchError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let reply: WireReply =
        serde_json::from_slice(bytes).map_err(|source| PerplexitySearchError::Reply { source })?;
    let results = reply
        .results
        .into_iter()
        .map(|result| SearchResult {
            title: result.title,
            url: result.url,
            snippet: result.snippet,
            date: result.date,
            last_updated: result.last_updated,
            extra: Object::new(),
        })
        .collect();
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}
