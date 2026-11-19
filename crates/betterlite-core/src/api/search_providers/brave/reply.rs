use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::value::RawValue;
use url::form_urlencoded;

use super::BraveSearchError;
use super::dates::{self, DateFlags, DateValue};

pub const REPLY_MAX_BYTES: usize = 0x0100_0000;

const SECTIONS: [&str; 6] = ["web", "discussions", "faqs", "faq", "news", "videos"];
const RESULT_FILTER: &str = "result_filter";
const COUNT: &str = "count";
const DEFAULT_COUNT: usize = 20;
const COUNT_FLOOR: i64 = 1;
const COUNT_LIMIT: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occurrence {
    First,
    Last,
}

// the sections the source reads by name, and only the member each one's results live in
#[derive(Debug, Deserialize)]
struct BraveReply {
    #[serde(default)]
    web: Option<SectionBody>,
    #[serde(default)]
    discussions: Option<SectionBody>,
    #[serde(default)]
    faqs: Option<SectionBody>,
    #[serde(default)]
    faq: Option<SectionBody>,
    #[serde(default)]
    news: Option<SectionBody>,
    #[serde(default)]
    videos: Option<SectionBody>,
}

#[derive(Debug, Deserialize)]
struct SectionBody {
    #[serde(default)]
    results: Option<Vec<Box<RawValue>>>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    #[serde(default)]
    title: Option<Box<RawValue>>,
    #[serde(default)]
    url: Option<Box<RawValue>>,
    #[serde(default)]
    description: Option<Box<RawValue>>,
    #[serde(default)]
    page_age: Option<Box<RawValue>>,
    #[serde(default)]
    age: Option<Box<RawValue>>,
    #[serde(default)]
    fetched_content_timestamp: Option<Box<RawValue>>,
}

pub fn decode_brave_search(
    bytes: &[u8],
    url: Option<&str>,
    today: NaiveDate,
) -> Result<SearchResponse, BraveSearchError> {
    if bytes.len() > REPLY_MAX_BYTES {
        return Err(BraveSearchError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
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
    let Some(raw) = query_parameter(url, RESULT_FILTER, Occurrence::Last) else {
        return SECTIONS.to_vec();
    };
    let mut selected = Vec::new();
    for part in raw.split(',') {
        let name = part.trim().to_lowercase();
        if let Some(section) = SECTIONS
            .iter()
            .copied()
            .find(|section| *section == name.as_str())
        {
            selected.push(section);
        }
    }
    if selected.is_empty() {
        SECTIONS.to_vec()
    } else {
        selected
    }
}

// dict(query_params) keeps a repeated parameter's last value; QueryParams.get keeps its first
fn query_parameter(url: Option<&str>, name: &str, occurrence: Occurrence) -> Option<String> {
    let query = url?.split_once('?').map(|(_, query)| query)?;
    let mut found = None;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key != name {
            continue;
        }
        if matches!(occurrence, Occurrence::First) {
            return Some(value.into_owned());
        }
        found = Some(value.into_owned());
    }
    found
}

fn result_limit(url: Option<&str>) -> Result<usize, BraveSearchError> {
    let Some(raw) = query_parameter(url, COUNT, Occurrence::First) else {
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
    let result: BraveResult =
        serde_json::from_str(raw.get()).map_err(BraveSearchError::ReplyDecode)?;
    let title = text_field(result.title, "title")?;
    let url = text_field(result.url, "url")?;
    let snippet = text_field(result.description, "description")?;
    let sources = [
        result
            .page_age
            .as_deref()
            .map(|raw| DateValue::from_literal(raw.get())),
        result
            .age
            .as_deref()
            .map(|raw| DateValue::from_literal(raw.get())),
    ];
    let date = projected_date(sources, today);
    let last_updated = result
        .fetched_content_timestamp
        .as_deref()
        .map(|raw| DateValue::from_literal(raw.get()))
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

// the source's `page_age or age` takes the first member JSON truthiness keeps
fn projected_date(sources: [Option<DateValue>; 2], today: NaiveDate) -> Option<String> {
    let [page_age, age] = sources;
    let value = page_age
        .filter(DateValue::is_filled)
        .or(age.filter(DateValue::is_filled))?;
    dates::to_yyyy_mm_dd(&value, today, DateFlags::default())
}

fn text_field(
    member: Option<Box<RawValue>>,
    field: &'static str,
) -> Result<String, BraveSearchError> {
    let Some(raw) = member else {
        return Ok(String::new());
    };
    let text = raw.get().trim_ascii();
    if text == "null" {
        return Ok(String::new());
    }
    serde_json::from_str(text).map_err(|source| BraveSearchError::ReplyText { field, source })
}
