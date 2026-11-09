use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde_json::value::RawValue;

use super::SerperSearchError;

pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;

#[derive(Debug, Deserialize)]
struct SerperReply {
    #[serde(default)]
    organic: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct SerperReplyResult {
    #[serde(default)]
    title: Option<Box<RawValue>>,
    #[serde(default)]
    link: Option<Box<RawValue>>,
    #[serde(default)]
    snippet: Option<Box<RawValue>>,
    #[serde(default)]
    date: Option<Box<RawValue>>,
}

pub fn decode_serper_search(bytes: &[u8]) -> Result<SearchResponse, SerperSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(SerperSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let reply: SerperReply =
        serde_json::from_slice(bytes).map_err(SerperSearchError::ReplyDecode)?;
    let results = match reply.organic {
        None => Vec::new(),
        Some(raw) => {
            if serde_json::from_str::<()>(raw.get()).is_ok() {
                return Err(SerperSearchError::OrganicShape);
            }
            let items: Vec<SerperReplyResult> =
                serde_json::from_str(raw.get()).map_err(SerperSearchError::ReplyDecode)?;
            items
                .into_iter()
                .map(serper_result)
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

fn serper_result(result: SerperReplyResult) -> Result<SearchResult, SerperSearchError> {
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.link, "link")?,
        snippet: text_member(result.snippet, "snippet")?,
        date: date_member(result.date, "date")?,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<String, SerperSearchError> {
    let Some(raw) = value else {
        return Ok(String::new());
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Err(SerperSearchError::ResultField { field: name });
    }
    serde_json::from_str(raw.get()).map_err(|source| SerperSearchError::ReplyField {
        field: name,
        source,
    })
}

fn date_member(
    value: Option<Box<RawValue>>,
    name: &'static str,
) -> Result<Option<String>, SerperSearchError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    if serde_json::from_str::<()>(raw.get()).is_ok() {
        return Ok(None);
    }
    serde_json::from_str(raw.get())
        .map(Some)
        .map_err(|source| SerperSearchError::ReplyField {
            field: name,
            source,
        })
}
