use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde_json::Deserializer;

use super::error::TavilyError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

#[derive(Deserialize)]
struct TavilyReply {
    #[serde(default)]
    results: Vec<TavilyReplyResult>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

#[derive(Deserialize)]
struct TavilyReplyResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

pub fn decode_tavily_reply(bytes: &[u8]) -> Result<SearchResponse, TavilyError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(TavilyError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut decoder = Deserializer::from_slice(bytes);
    let reply: TavilyReply = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| TavilyError::Reply { source })?;
    decoder.end().map_err(TavilyError::TrailingBytes)?;
    let results = reply.results.into_iter().map(project).collect();
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn project(result: TavilyReplyResult) -> SearchResult {
    SearchResult {
        title: result.title,
        url: result.url,
        snippet: result.content,
        date: None,
        last_updated: None,
        extra: Object::new(),
    }
}
