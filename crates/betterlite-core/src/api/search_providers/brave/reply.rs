use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::value::RawValue;
use url::form_urlencoded;

use super::BraveSearchError;
use super::dates::{self, DateFlags, DateValue};

const SECTIONS: [&str; 6] = ["web", "discussions", "faqs", "faq", "news", "videos"];
const RESULT_FILTER: &str = "result_filter";
const COUNT: &str = "count";
const DEFAULT_COUNT: usize = 20;
const COUNT_FLOOR: i64 = 1;
const COUNT_LIMIT: i64 = 20;

// the sections the source reads by name, and only the member each one's results live in
#[derive(Debug, Deserialize)]
struct BraveReply {
    web: Option<SectionBody>,
    discussions: Option<SectionBody>,
    faqs: Option<SectionBody>,
    faq: Option<SectionBody>,
    news: Option<SectionBody>,
    videos: Option<SectionBody>,
}

#[derive(Debug, Deserialize)]
struct SectionBody {
    results: Option<Vec<Box<RawValue>>>,
}

pub fn decode_brave_search(
    bytes: &[u8],
    url: Option<&str>,
    today: NaiveDate,
) -> Result<SearchResponse, BraveSearchError> {
    let reply: BraveReply = serde_json::from_slice(bytes).map_err(BraveSearchError::ReplyDecode)?;
    let limit = result_limit(url)?;
    let mut results = Vec::new();
    for section in sections(url) {
        let Some(items) = section_results(&reply, section) else {
            continue;
        };
        for raw in items {
            if results.len() >= limit {
                break;
            }
            results.push(project(raw, today)?);
        }
    }
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn section_results<'reply>(
    reply: &'reply BraveReply,
    section: &str,
) -> Option<&'reply Vec<Box<RawValue>>> {
    let body = match section {
        "web" => reply.web.as_ref(),
        "discussions" => reply.discussions.as_ref(),
        "faqs" => reply.faqs.as_ref(),
        "faq" => reply.faq.as_ref(),
        "news" => reply.news.as_ref(),
        "videos" => reply.videos.as_ref(),
        _ => None,
    }?;
    body.results.as_ref()
}

// an unsupported or empty filter selects every section, in the source's own order
fn sections(url: Option<&str>) -> Vec<&'static str> {
    let raw = query_parameter(url, RESULT_FILTER).1.unwrap_or_default();
    let selected: Vec<&'static str> = raw
        .split(',')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter_map(|name| SECTIONS.iter().copied().find(|section| *section == name))
        .collect();
    if selected.is_empty() {
        SECTIONS.to_vec()
    } else {
        selected
    }
}

// dict(query_params) keeps a repeated parameter's last value; QueryParams.get keeps its first
fn query_parameter(url: Option<&str>, name: &str) -> (Option<String>, Option<String>) {
    let mut first = None;
    let mut last = None;
    if let Some(query) = url
        .and_then(|text| text.split_once('?'))
        .map(|(_, query)| query)
    {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            if key == name {
                first = first.or_else(|| Some(value.as_ref().to_owned()));
                last = Some(value.into_owned());
            }
        }
    }
    (first, last)
}

fn result_limit(url: Option<&str>) -> Result<usize, BraveSearchError> {
    let Some(raw) = query_parameter(url, COUNT).0 else {
        return Ok(DEFAULT_COUNT);
    };
    let count = raw
        .trim()
        .parse::<i64>()
        .map_err(|_error| BraveSearchError::CountNotInteger { value: raw.clone() })?;
    usize::try_from(count.clamp(COUNT_FLOOR, COUNT_LIMIT))
        .map_err(|_error| BraveSearchError::CountNotInteger { value: raw })
}

fn project(raw: &RawValue, today: NaiveDate) -> Result<SearchResult, BraveSearchError> {
    // the source reads each member by name, so a map keeps every member it might ask for
    let result: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(raw.get()).map_err(BraveSearchError::ReplyDecode)?;
    let title = text_field(result.get("title").map(Box::as_ref), "title")?;
    let url = text_field(result.get("url").map(Box::as_ref), "url")?;
    let snippet = text_field(result.get("description").map(Box::as_ref), "description")?;
    // the source's `page_age or age` takes the first member JSON truthiness keeps
    let [page_age, age] = [result.get("page_age"), result.get("age")]
        .map(|member| member.map(|item| DateValue::from_literal(item.get())));
    let date = page_age
        .filter(DateValue::is_filled)
        .or(age.filter(DateValue::is_filled))
        .and_then(|value| dates::to_yyyy_mm_dd(&value, today, DateFlags::default()));
    let last_updated = result
        .get("fetched_content_timestamp")
        .map(|item| DateValue::from_literal(item.get()))
        .and_then(|value| dates::to_yyyy_mm_dd(&value, today, DateFlags::default()));
    Ok(SearchResult {
        title,
        url,
        snippet,
        date,
        last_updated,
        extra: Object::new(),
    })
}

fn text_field(member: Option<&RawValue>, field: &'static str) -> Result<String, BraveSearchError> {
    let Some(raw) = member else {
        return Ok(String::new());
    };
    let text = raw.get().trim_ascii();
    if text == "null" {
        return Ok(String::new());
    }
    serde_json::from_str(text).map_err(|source| BraveSearchError::ReplyText { field, source })
}
