use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde::de::DeserializeOwned;
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
    let reply: SerperReply = parse_object(bytes)?;
    let results = match reply.organic {
        None => Vec::new(),
        Some(raw) => {
            if serde_json::from_str::<()>(raw.get()).is_ok() {
                return Err(SerperSearchError::OrganicShape);
            }
            let items: Vec<Box<RawValue>> =
                serde_json::from_str(raw.get()).map_err(SerperSearchError::ReplyDecode)?;
            let mut results = Vec::with_capacity(items.len());
            for item in &items {
                results.push(serper_result(parse_object(item.get().as_bytes())?)?);
            }
            results
        }
    };
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

// serde fills a struct from a JSON array too, and the source fails on anything but an object
fn parse_object<T>(bytes: &[u8]) -> Result<T, SerperSearchError>
where
    T: DeserializeOwned,
{
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(SerperSearchError::NotAnObject);
    }
    serde_json::from_slice(bytes).map_err(SerperSearchError::ReplyDecode)
}

fn serper_result(result: SerperReplyResult) -> Result<SearchResult, SerperSearchError> {
    // a missing date and a null date are both absent; a present non-string date fails like the source
    let date = result
        .date
        .filter(|raw| serde_json::from_str::<()>(raw.get()).is_err())
        .map(|raw| parse_text(&raw, "date"))
        .transpose()?;
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.link, "link")?,
        snippet: text_member(result.snippet, "snippet")?,
        date,
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
    parse_text(&raw, name)
}

fn parse_text(raw: &RawValue, name: &'static str) -> Result<String, SerperSearchError> {
    serde_json::from_str(raw.get()).map_err(|source| SerperSearchError::ReplyField {
        field: name,
        source,
    })
}
