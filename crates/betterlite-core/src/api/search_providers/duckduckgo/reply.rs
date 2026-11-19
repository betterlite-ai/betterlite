use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_with::rust::double_option;

use super::DuckDuckGoSearchError;
use super::request::DuckDuckGoParams;

pub const REPLY_MAX_BYTES: usize = 0x0100_0000;

// outer None is a missing member and inner None is a null one, so the two stay apart
type Member = Option<Option<Box<RawValue>>>;

#[derive(Clone, Copy)]
enum Level {
    RelatedTopics,
    Nested,
}

#[derive(Debug, Deserialize)]
struct DuckDuckGoReply {
    #[serde(default, rename = "AbstractURL", with = "double_option")]
    abstract_url: Member,
    #[serde(default, rename = "AbstractText", with = "double_option")]
    abstract_text: Member,
    #[serde(default, rename = "Heading", with = "double_option")]
    heading: Member,
    #[serde(default, rename = "RelatedTopics", with = "double_option")]
    related_topics: Member,
}

#[derive(Debug, Deserialize)]
struct RelatedTopic {
    #[serde(default, rename = "FirstURL", with = "double_option")]
    first_url: Member,
    #[serde(default, rename = "Text", with = "double_option")]
    text: Member,
    #[serde(default, rename = "Topics", with = "double_option")]
    topics: Member,
}

pub fn decode_duckduckgo_search(
    bytes: &[u8],
    params: &DuckDuckGoParams,
) -> Result<SearchResponse, DuckDuckGoSearchError> {
    let DuckDuckGoReply {
        abstract_url,
        abstract_text,
        heading,
        related_topics,
    } = parse_reply(bytes)?;
    let limit = limit_of(params);
    let mut results = Vec::new();
    push_abstract(abstract_url, abstract_text, heading, &mut results)?;
    let topics = topics_of(related_topics)?;
    append_entries(&topics, Level::RelatedTopics, limit, &mut results)?;
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn parse_reply(bytes: &[u8]) -> Result<DuckDuckGoReply, DuckDuckGoSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(DuckDuckGoSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    parse_object::<DuckDuckGoReply>(bytes)
}

fn limit_of(params: &DuckDuckGoParams) -> Option<i64> {
    match params.max_results.as_ref() {
        Some(JsonValue::Number(number)) => number.as_i64(),
        Some(JsonValue::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn topics_of(member: Member) -> Result<Vec<Box<RawValue>>, DuckDuckGoSearchError> {
    match member {
        None => Ok(Vec::new()),
        Some(None) => Err(DuckDuckGoSearchError::ReplyNull {
            field: "RelatedTopics",
        }),
        Some(Some(raw)) => parse_list(&raw, "RelatedTopics"),
    }
}

fn append_entries(
    entries: &[Box<RawValue>],
    level: Level,
    limit: Option<i64>,
    results: &mut Vec<SearchResult>,
) -> Result<(), DuckDuckGoSearchError> {
    for entry in entries {
        if matches!(level, Level::RelatedTopics)
            && entry.get().trim_ascii_start().as_bytes().first() != Some(&b'{')
        {
            continue;
        }
        if limit_reached(results.len(), limit) {
            break;
        }
        append_entry(entry.get().as_bytes(), level, limit, results)?;
    }
    Ok(())
}

fn append_entry(
    bytes: &[u8],
    level: Level,
    limit: Option<i64>,
    results: &mut Vec<SearchResult>,
) -> Result<(), DuckDuckGoSearchError> {
    let mut topic = parse_entry(bytes)?;
    if let Some(result) = direct_topic(&mut topic)? {
        results.push(result);
        return Ok(());
    }
    if matches!(level, Level::Nested) {
        return Ok(());
    }
    let Some(nested) = &topic.topics else {
        return Ok(());
    };
    let Some(raw) = nested else {
        return Err(DuckDuckGoSearchError::ReplyNull { field: "Topics" });
    };
    let entries = parse_list(raw, "Topics")?;
    append_entries(&entries, Level::Nested, limit, results)
}

fn push_abstract(
    abstract_url: Member,
    abstract_text: Member,
    heading: Member,
    results: &mut Vec<SearchResult>,
) -> Result<(), DuckDuckGoSearchError> {
    let Some(url) = text_member(abstract_url, "AbstractURL")? else {
        return Ok(());
    };
    if url.is_empty() {
        return Ok(());
    }
    let Some(snippet) = text_member(abstract_text, "AbstractText")? else {
        return Ok(());
    };
    if snippet.is_empty() {
        return Ok(());
    }
    let title = match heading {
        None => String::new(),
        Some(None) => return Err(DuckDuckGoSearchError::ReplyNull { field: "Heading" }),
        Some(Some(raw)) => parse_text(&raw, "Heading")?,
    };
    results.push(SearchResult {
        title,
        url,
        snippet,
        date: None,
        last_updated: None,
        extra: Object::new(),
    });
    Ok(())
}

fn direct_topic(entry: &mut RelatedTopic) -> Result<Option<SearchResult>, DuckDuckGoSearchError> {
    if entry.first_url.is_none() || entry.text.is_none() {
        return Ok(None);
    }
    let Some(url) = text_member(entry.first_url.take(), "FirstURL")? else {
        return Err(DuckDuckGoSearchError::ReplyNull { field: "FirstURL" });
    };
    let Some(text) = text_member(entry.text.take(), "Text")? else {
        return Err(DuckDuckGoSearchError::ReplyNull { field: "Text" });
    };
    Ok(Some(topic_result(&text, &url)))
}

fn topic_result(text: &str, url: &str) -> SearchResult {
    let (title, snippet) = split_title(text);
    SearchResult {
        title,
        url: url.to_owned(),
        snippet,
        date: None,
        last_updated: None,
        extra: Object::new(),
    }
}

fn split_title(text: &str) -> (String, String) {
    match text.split_once(" - ") {
        Some((title, snippet)) => (title.to_owned(), snippet.to_owned()),
        None => (truncated_title(text), text.to_owned()),
    }
}

fn truncated_title(text: &str) -> String {
    let mut characters = text.chars();
    let title: String = characters.by_ref().take(50).collect();
    if characters.next().is_some() {
        format!("{title}...")
    } else {
        title
    }
}

fn limit_reached(count: usize, limit: Option<i64>) -> bool {
    let Some(maximum) = limit else {
        return false;
    };
    i64::try_from(count).map_or(true, |number| number >= maximum)
}

fn text_member(
    member: Member,
    field: &'static str,
) -> Result<Option<String>, DuckDuckGoSearchError> {
    match member {
        Some(Some(raw)) => parse_text(&raw, field).map(Some),
        Some(None) | None => Ok(None),
    }
}

fn parse_text(raw: &RawValue, field: &'static str) -> Result<String, DuckDuckGoSearchError> {
    serde_json::from_str(raw.get())
        .map_err(|source| DuckDuckGoSearchError::ReplyText { field, source })
}

fn parse_list(
    raw: &RawValue,
    field: &'static str,
) -> Result<Vec<Box<RawValue>>, DuckDuckGoSearchError> {
    serde_json::from_str(raw.get())
        .map_err(|source| DuckDuckGoSearchError::ReplyList { field, source })
}

fn parse_entry(bytes: &[u8]) -> Result<RelatedTopic, DuckDuckGoSearchError> {
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(DuckDuckGoSearchError::NestedEntry);
    }
    serde_json::from_slice(bytes).map_err(|source| DuckDuckGoSearchError::ReplyJson { source })
}

fn parse_object<T>(bytes: &[u8]) -> Result<T, DuckDuckGoSearchError>
where
    T: DeserializeOwned,
{
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(DuckDuckGoSearchError::ReplyNotObject);
    }
    serde_json::from_slice(bytes).map_err(|source| DuckDuckGoSearchError::ReplyJson { source })
}
