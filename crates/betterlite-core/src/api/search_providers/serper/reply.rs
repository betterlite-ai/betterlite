use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_with::rust::double_option;

use super::SerperSearchError;

pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;

// outer None is a missing member and inner None is a null one, so the two stay apart
type Member = Option<Option<Box<RawValue>>>;

#[derive(Debug, Deserialize)]
struct SerperReply {
    #[serde(default, with = "double_option")]
    organic: Member,
}

#[derive(Debug, Deserialize)]
struct SerperReplyResult {
    #[serde(default, with = "double_option")]
    title: Member,
    #[serde(default, with = "double_option")]
    link: Member,
    #[serde(default, with = "double_option")]
    snippet: Member,
    #[serde(default, with = "double_option")]
    date: Member,
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
        Some(None) => return Err(SerperSearchError::OrganicShape),
        Some(Some(raw)) => {
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
    let date = match result.date {
        Some(Some(raw)) => Some(parse_text(&raw, "date")?),
        Some(None) | None => None,
    };
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.link, "link")?,
        snippet: text_member(result.snippet, "snippet")?,
        date,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(value: Member, name: &'static str) -> Result<String, SerperSearchError> {
    match value {
        None => Ok(String::new()),
        Some(None) => Err(SerperSearchError::ResultField { field: name }),
        Some(Some(raw)) => parse_text(&raw, name),
    }
}

fn parse_text(raw: &RawValue, name: &'static str) -> Result<String, SerperSearchError> {
    serde_json::from_str(raw.get()).map_err(|source| SerperSearchError::ReplyField {
        field: name,
        source,
    })
}
