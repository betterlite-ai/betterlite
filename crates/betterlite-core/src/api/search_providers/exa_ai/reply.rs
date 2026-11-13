use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_with::rust::double_option;

use super::ExaAiSearchError;

pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;

// outer None is a missing member and inner None is a null one, so the two stay apart
type Member = Option<Option<Box<RawValue>>>;

#[derive(Debug, Deserialize)]
struct ExaReply {
    #[serde(default, with = "double_option")]
    results: Member,
}

#[derive(Debug, Deserialize)]
struct ExaReplyResult {
    #[serde(default, with = "double_option")]
    title: Member,
    #[serde(default, with = "double_option")]
    url: Member,
    #[serde(default, with = "double_option")]
    text: Member,
    #[serde(default, rename = "publishedDate", with = "double_option")]
    published_date: Member,
}

pub fn decode_exa_ai_search(bytes: &[u8]) -> Result<SearchResponse, ExaAiSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(ExaAiSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let reply: ExaReply = parse_object(bytes)?;
    let results = match reply.results {
        None => Vec::new(),
        Some(None) => return Err(ExaAiSearchError::ResultsShape),
        Some(Some(raw)) => {
            let items: Vec<Box<RawValue>> =
                serde_json::from_str(raw.get()).map_err(ExaAiSearchError::ReplyDecode)?;
            let mut results = Vec::with_capacity(items.len());
            for item in &items {
                results.push(exa_result(parse_object(item.get().as_bytes())?)?);
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
fn parse_object<T>(bytes: &[u8]) -> Result<T, ExaAiSearchError>
where
    T: DeserializeOwned,
{
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(ExaAiSearchError::NotAnObject);
    }
    serde_json::from_slice(bytes).map_err(ExaAiSearchError::ReplyDecode)
}

fn exa_result(result: ExaReplyResult) -> Result<SearchResult, ExaAiSearchError> {
    // a missing date and a null date are both absent; a present non-string date fails like the source
    let date = match result.published_date {
        Some(Some(raw)) => Some(parse_text(&raw, "publishedDate")?),
        Some(None) | None => None,
    };
    Ok(SearchResult {
        title: text_member(result.title, "title")?,
        url: text_member(result.url, "url")?,
        snippet: text_member(result.text, "text")?,
        date,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(value: Member, name: &'static str) -> Result<String, ExaAiSearchError> {
    match value {
        None => Ok(String::new()),
        Some(None) => Err(ExaAiSearchError::ResultField { field: name }),
        Some(Some(raw)) => parse_text(&raw, name),
    }
}

fn parse_text(raw: &RawValue, name: &'static str) -> Result<String, ExaAiSearchError> {
    serde_json::from_str(raw.get()).map_err(|source| ExaAiSearchError::ReplyField {
        field: name,
        source,
    })
}
