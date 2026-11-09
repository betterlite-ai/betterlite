use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde_json::value::RawValue;

use super::ExaAiSearchError;

pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;

#[derive(Debug, Deserialize)]
struct ExaReply {
    #[serde(default)]
    results: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct ExaReplyResult {
    #[serde(default)]
    title: Option<Box<RawValue>>,
    #[serde(default)]
    url: Option<Box<RawValue>>,
    #[serde(default)]
    text: Option<Box<RawValue>>,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<Box<RawValue>>,
}

pub fn decode_exa_ai_search(bytes: &[u8]) -> Result<SearchResponse, ExaAiSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(ExaAiSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let reply: ExaReply = serde_json::from_slice(bytes).map_err(ExaAiSearchError::ReplyDecode)?;
    let results = match reply.results {
        None => Vec::new(),
        Some(raw) => {
            if serde_json::from_str::<()>(raw.get()).is_ok() {
                return Err(ExaAiSearchError::ResultsShape);
            }
            let items: Vec<ExaReplyResult> =
                serde_json::from_str(raw.get()).map_err(ExaAiSearchError::ReplyDecode)?;
            items
                .into_iter()
                .map(exa_result)
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn exa_result(result: ExaReplyResult) -> Result<SearchResult, ExaAiSearchError> {
    // a missing date and a null date are both absent; a present non-string date fails like the source
    let date = result
        .published_date
        .filter(|raw| serde_json::from_str::<()>(raw.get()).is_err())
        .map(|raw| parse_text(&raw, "publishedDate"))
        .transpose()?;
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.url, "url")?,
        snippet: text_member(result.text, "text")?,
        date,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<String, ExaAiSearchError> {
    let Some(raw) = value else {
        return Ok(String::new());
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Err(ExaAiSearchError::ResultField { field: name });
    }
    parse_text(&raw, name)
}

fn parse_text(raw: &RawValue, name: &'static str) -> Result<String, ExaAiSearchError> {
    serde_json::from_str(raw.get()).map_err(|source| ExaAiSearchError::ReplyField {
        field: name,
        source,
    })
}
