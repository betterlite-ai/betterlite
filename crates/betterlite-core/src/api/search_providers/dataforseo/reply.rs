use std::cmp::Ordering;

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_with::rust::double_option;

use super::DataForSeoSearchError;

pub(super) const REPLY_MAX_BYTES: usize = 0x0100_0000;
const SUCCESS_STATUS: f64 = 20_000.0;

// outer None is a missing member and inner None is a null one, so the two stay apart
type Member = Option<Option<Box<RawValue>>>;

#[derive(Debug, Deserialize)]
struct DataForSeoReply {
    #[serde(default, with = "double_option")]
    tasks: Member,
}

#[derive(Debug, Deserialize)]
struct DataForSeoTask {
    #[serde(default, with = "double_option")]
    status_code: Member,
    #[serde(default, with = "double_option")]
    result: Member,
}

#[derive(Debug, Deserialize)]
struct DataForSeoResult {
    #[serde(default, with = "double_option")]
    items: Member,
}

#[derive(Debug, Deserialize)]
struct DataForSeoItem {
    #[serde(default, rename = "type", with = "double_option")]
    kind: Member,
    #[serde(default, with = "double_option")]
    title: Member,
    #[serde(default, with = "double_option")]
    url: Member,
    #[serde(default, with = "double_option")]
    description: Member,
}

pub fn decode_dataforseo_search(bytes: &[u8]) -> Result<SearchResponse, DataForSeoSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(DataForSeoSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    let reply: DataForSeoReply = parse_object(bytes)?;
    let Some(tasks) = required(reply.tasks, DataForSeoSearchError::TasksShape)? else {
        return Ok(response(Vec::new()));
    };
    let Some(task) = first_element(&tasks)? else {
        return Ok(response(Vec::new()));
    };
    let parsed_task: DataForSeoTask = parse_object(task.get().as_bytes())?;
    Ok(response(task_hits(parsed_task)?))
}

fn task_hits(task: DataForSeoTask) -> Result<Vec<SearchResult>, DataForSeoSearchError> {
    if !succeeded(&task.status_code) {
        return Ok(Vec::new());
    }
    let Some(results) = required(task.result, DataForSeoSearchError::ResultShape)? else {
        return Ok(Vec::new());
    };
    let Some(result) = first_element(&results)? else {
        return Ok(Vec::new());
    };
    let parsed_result: DataForSeoResult = parse_object(result.get().as_bytes())?;
    result_hits(parsed_result)
}

fn result_hits(result: DataForSeoResult) -> Result<Vec<SearchResult>, DataForSeoSearchError> {
    let Some(items) = required(result.items, DataForSeoSearchError::ItemsShape)? else {
        return Ok(Vec::new());
    };
    let parsed_items: Vec<Box<RawValue>> =
        serde_json::from_str(items.get()).map_err(DataForSeoSearchError::ReplyDecode)?;
    let mut hits = Vec::with_capacity(parsed_items.len());
    for item in &parsed_items {
        let parsed_item: DataForSeoItem = parse_object(item.get().as_bytes())?;
        if organic(&parsed_item.kind) {
            hits.push(project_item(parsed_item)?);
        }
    }
    Ok(hits)
}

fn required(
    member: Member,
    shape: DataForSeoSearchError,
) -> Result<Option<Box<RawValue>>, DataForSeoSearchError> {
    match member {
        None => Ok(None),
        Some(None) => Err(shape),
        Some(Some(raw)) => Ok(Some(raw)),
    }
}

fn first_element(raw: &RawValue) -> Result<Option<Box<RawValue>>, DataForSeoSearchError> {
    let elements: Vec<Box<RawValue>> =
        serde_json::from_str(raw.get()).map_err(DataForSeoSearchError::ReplyDecode)?;
    Ok(elements.into_iter().next())
}

// the source compares status_code to the integer 20000, so a float of the same value counts and a string does not
fn succeeded(status: &Member) -> bool {
    let Some(Some(raw)) = status else {
        return false;
    };
    serde_json::from_str::<f64>(raw.get())
        .is_ok_and(|number| number.total_cmp(&SUCCESS_STATUS) == Ordering::Equal)
}

fn organic(kind: &Member) -> bool {
    let Some(Some(raw)) = kind else {
        return false;
    };
    serde_json::from_str::<String>(raw.get()).is_ok_and(|text| text == "organic")
}

// serde fills a struct from a JSON array too, and the source fails on anything but an object
fn parse_object<T>(bytes: &[u8]) -> Result<T, DataForSeoSearchError>
where
    T: DeserializeOwned,
{
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(DataForSeoSearchError::NotAnObject);
    }
    serde_json::from_slice(bytes).map_err(DataForSeoSearchError::ReplyDecode)
}

fn project_item(item: DataForSeoItem) -> Result<SearchResult, DataForSeoSearchError> {
    Ok(SearchResult {
        title: text_member(item.title, "title")?,
        url: text_member(item.url, "url")?,
        snippet: text_member(item.description, "description")?,
        date: None,
        last_updated: None,
        extra: Object::new(),
    })
}

fn text_member(value: Member, name: &'static str) -> Result<String, DataForSeoSearchError> {
    match value {
        None => Ok(String::new()),
        Some(None) => Err(DataForSeoSearchError::ResultField { field: name }),
        Some(Some(raw)) => parse_text(&raw, name),
    }
}

fn parse_text(raw: &RawValue, name: &'static str) -> Result<String, DataForSeoSearchError> {
    serde_json::from_str(raw.get()).map_err(|source| DataForSeoSearchError::ReplyField {
        field: name,
        source,
    })
}

fn response(results: Vec<SearchResult>) -> SearchResponse {
    SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    }
}
