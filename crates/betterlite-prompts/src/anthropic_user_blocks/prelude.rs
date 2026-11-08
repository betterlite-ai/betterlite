use thiserror::Error;

use crate::anthropic_history::HistoryError;
use crate::anthropic_history::text::sanitize_empty_text_content;
use crate::anthropic_history::tools::{MAX_HISTORY_MESSAGES, sanitize_messages};
use crate::arguments::json::{Json, Object};
use crate::media::limits::{MAX_MESSAGE_PARTS, MAX_REMOTE_URLS};

const CONTINUE_TEXT: &str = "Please continue.";
const CONTENT: &str = "content";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FUNCTION: &str = "function";
const IMAGE_URL: &str = "image_url";
const ROLE: &str = "role";
const TOOL: &str = "tool";
const TYPE: &str = "type";
const URL: &str = "url";
const USER: &str = "user";

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(
        "Anthropic needs at least one non-system message; set modify_params to add the dummy user message"
    )]
    NoUserMessages,
    #[error("messages exceeds the maximum of {limit}")]
    TooManyMessages { limit: usize },
    #[error("a content list exceeds the maximum of {limit} parts")]
    TooManyParts { limit: usize },
    #[error("a request would download more than {limit} remote files")]
    TooManyRemoteUrls { limit: usize },
    #[error("message at index {index} is not an object")]
    MessageNotObject { index: usize },
}

#[derive(Debug)]
pub struct Prepared {
    pub messages: Vec<Json>,
    pub force_base64: bool,
}

pub fn prepare(
    messages: &[Json],
    model: &str,
    llm_provider: &str,
    modify_params: bool,
) -> Result<Prepared, PrepareError> {
    check_bounds(messages)?;
    let sanitized = sanitize_messages(messages, modify_params)?;
    let mut prepared: Vec<Json> = sanitized.iter().map(sanitize_empty_text_content).collect();
    if prepared.is_empty() {
        if !modify_params {
            return Err(PrepareError::NoUserMessages);
        }
        prepared.push(dummy_message());
    }
    let force_base64 =
        model.to_lowercase().starts_with("invoke/") || llm_provider.starts_with("vertex_ai");
    if count_downloads(&prepared, force_base64) > MAX_REMOTE_URLS {
        return Err(PrepareError::TooManyRemoteUrls {
            limit: MAX_REMOTE_URLS,
        });
    }
    Ok(Prepared {
        messages: prepared,
        force_base64,
    })
}

fn check_bounds(messages: &[Json]) -> Result<(), PrepareError> {
    if messages.len() > MAX_HISTORY_MESSAGES {
        return Err(PrepareError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES,
        });
    }
    for (index, message) in messages.iter().enumerate() {
        let Some(fields) = message.as_object() else {
            return Err(PrepareError::MessageNotObject { index });
        };
        if let Some(Json::List(parts)) = fields.get(CONTENT)
            && parts.len() > MAX_MESSAGE_PARTS
        {
            return Err(PrepareError::TooManyParts {
                limit: MAX_MESSAGE_PARTS,
            });
        }
    }
    Ok(())
}

// The conversion's own predicates: http:// always downloads, https:// only for force_base64
fn count_downloads(messages: &[Json], force_base64: bool) -> usize {
    let mut downloads: usize = 0;
    for message in messages {
        let Some(fields) = message.as_object() else {
            continue;
        };
        let role = fields.get(ROLE).and_then(Json::as_str).unwrap_or_default();
        if !matches!(role, USER | TOOL | FUNCTION) {
            continue;
        }
        let Some(Json::List(parts)) = fields.get(CONTENT) else {
            continue;
        };
        for part in parts {
            let Some(part_fields) = part.as_object() else {
                continue;
            };
            if part_downloads(part_fields, force_base64) {
                downloads = downloads.saturating_add(1);
            }
        }
    }
    downloads
}

fn part_downloads(part: &Object, force_base64: bool) -> bool {
    match part.get(TYPE).and_then(Json::as_str).unwrap_or_default() {
        IMAGE_URL => part
            .get(IMAGE_URL)
            .is_some_and(|url| url_downloads(url, force_base64)),
        FILE => part
            .get(FILE)
            .and_then(Json::as_object)
            .and_then(|file| file.get(FILE_DATA))
            .and_then(Json::as_str)
            .is_some_and(|data| data.starts_with("http")),
        _ => false,
    }
}

fn url_downloads(url: &Json, force_base64: bool) -> bool {
    let address = match url {
        Json::Text(text) => Some(text.as_str()),
        Json::Object(fields) => fields.get(URL).and_then(Json::as_str),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    };
    address.is_some_and(|text| {
        text.starts_with("http://") || (force_base64 && text.starts_with("https://"))
    })
}

fn dummy_message() -> Json {
    Json::Object(Object::from_iter([
        (ROLE.to_owned(), Json::Text(USER.to_owned())),
        (CONTENT.to_owned(), Json::Text(CONTINUE_TEXT.to_owned())),
    ]))
}
