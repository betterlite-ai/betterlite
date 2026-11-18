use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde_json::Number;
use serde_json::value::RawValue;
use serde_with::rust::double_option;

use super::ApiserpentSearchError;

pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;

// outer None is a missing member and inner None is a null one, so the two stay apart
type Member = Option<Option<Box<RawValue>>>;

#[derive(Debug, Deserialize)]
struct ApiserpentReply {
    #[serde(default, with = "double_option")]
    results: Member,
}

#[derive(Debug, Deserialize)]
struct ApiserpentOrganic {
    #[serde(default, with = "double_option")]
    organic: Member,
}

#[derive(Debug, Deserialize)]
struct ApiserpentResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    date: Option<String>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

pub fn decode_apiserpent_search(bytes: &[u8]) -> Result<SearchResponse, ApiserpentSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(ApiserpentSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    if bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        != Some(b'{')
    {
        return Err(ApiserpentSearchError::NotAnObject);
    }
    let reply: ApiserpentReply =
        serde_json::from_slice(bytes).map_err(ApiserpentSearchError::ReplyDecode)?;
    let results = match reply.results {
        None | Some(None) => Vec::new(),
        Some(Some(raw)) if is_falsey(&raw) => Vec::new(),
        Some(Some(raw)) => results_from(&raw)?,
    };
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn results_from(raw: &RawValue) -> Result<Vec<SearchResult>, ApiserpentSearchError> {
    match raw.get().trim_ascii_start().as_bytes().first().copied() {
        Some(b'[') => decode_results(raw.get()),
        Some(b'{') => organic_results(raw.get()),
        _ => Err(ApiserpentSearchError::ResultsShape),
    }
}

fn decode_results(text: &str) -> Result<Vec<SearchResult>, ApiserpentSearchError> {
    let items: Vec<ApiserpentResult> =
        serde_json::from_str(text).map_err(ApiserpentSearchError::ReplyDecode)?;
    Ok(items.into_iter().map(project).collect())
}

fn organic_results(text: &str) -> Result<Vec<SearchResult>, ApiserpentSearchError> {
    let reply: ApiserpentOrganic =
        serde_json::from_str(text).map_err(ApiserpentSearchError::ReplyDecode)?;
    match reply.organic {
        None => Ok(Vec::new()),
        Some(None) => Err(ApiserpentSearchError::OrganicShape),
        Some(Some(raw)) if empty_sequence(&raw) => Ok(Vec::new()),
        Some(Some(raw)) => decode_results(raw.get()),
    }
}

fn project(result: ApiserpentResult) -> SearchResult {
    SearchResult {
        title: result.title,
        url: result.url,
        snippet: result.snippet,
        date: result.date,
        last_updated: None,
        extra: Object::new(),
    }
}

// the source turns a falsey results value into {}, whose organic member is then absent
fn is_falsey(raw: &RawValue) -> bool {
    let text = raw.get().trim_ascii();
    match text.as_bytes().first().copied() {
        Some(b'f' | b'n') => true,
        Some(b'"' | b'[' | b'{') => empty_sequence(raw),
        _ => is_zero(text),
    }
}

// only a list, an object or a string can iterate zero times; the source fails on other falsey values
fn empty_sequence(raw: &RawValue) -> bool {
    let text = raw.get().trim_ascii();
    match text.as_bytes().first().copied() {
        Some(b'[') => {
            serde_json::from_str::<Vec<Box<RawValue>>>(text).is_ok_and(|items| items.is_empty())
        }
        Some(b'{') => serde_json::from_str::<Object>(text).is_ok_and(|fields| fields.is_empty()),
        Some(b'"') => text == "\"\"",
        _ => false,
    }
}

fn is_zero(text: &str) -> bool {
    serde_json::from_str::<Number>(text)
        .is_ok_and(|number| number.as_f64().is_some_and(|value| value == 0.0))
}
