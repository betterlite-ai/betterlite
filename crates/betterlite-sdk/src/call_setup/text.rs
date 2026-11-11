use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;
use betterlite_redact::structured::redact::redact_credentials_in_payload;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

use crate::call_setup::RequestMap;

const DEFAULT_MESSAGE_VALUE: &str = "default-message-value";

// A file handle cannot ride in a JSON request, so Rust callers pass it beside the document mapping
pub enum OcrFile {
    Path(PathBuf),
    Bytes(Vec<u8>),
    Named(String),
}

#[must_use]
pub fn ocr_document_summary(document: Option<&Json>, file: Option<&OcrFile>) -> String {
    let Some(Json::Object(fields)) = document else {
        return DEFAULT_MESSAGE_VALUE.to_owned();
    };
    if let Some(Json::Text(text)) = fields
        .get("document_url")
        .or_else(|| fields.get("image_url"))
    {
        if let Some((header, payload)) = text.split_once(',')
            && header.starts_with("data:")
        {
            return format!("{header} ({} chars)", payload.chars().count());
        }
        return text.clone();
    }
    let kind = match fields.get("mime_type") {
        Some(Json::Text(mime)) => format!("file ({mime})"),
        _ => String::from("file"),
    };
    match file {
        Some(OcrFile::Path(path)) => format!("{kind} {}", file_name(path)),
        Some(OcrFile::Bytes(bytes)) => format!("{kind} {} bytes", bytes.len()),
        Some(OcrFile::Named(name)) => format!("{kind} {}", file_name(Path::new(name))),
        None => kind,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned()
}

#[must_use]
pub fn get_logging_id(start_time: DateTime<FixedOffset>, response: &Json) -> Option<String> {
    let Json::Object(fields) = response else {
        return None;
    };
    let Some(Json::Text(id)) = fields.get("id") else {
        return None;
    };
    Some(format!(
        "time-{}-{:06}_{id}",
        start_time.format("%H-%M-%S"),
        start_time.timestamp_subsec_micros()
    ))
}

#[derive(Debug, Error)]
pub enum DebugTextError {
    #[error("the request could not be encoded for the debug line: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the encoded request is not an object")]
    Shape,
}

pub fn debug_text(
    original_function: &str,
    request: &RequestMap,
    debugging: bool,
) -> Result<Option<String>, DebugTextError> {
    if !debugging || inner_async_call(original_function, request) {
        return Ok(None);
    }
    let encoded = serde_json::to_value(Json::Object(request.clone()))?;
    let Some(fields) = encoded.as_object() else {
        return Err(DebugTextError::Shape);
    };
    let redacted = redact_credentials_in_payload(fields);
    let mut rendered = Vec::new();
    if let Some(redacted_fields) = redacted.as_object() {
        for (key, value) in redacted_fields {
            rendered.push(format!("{key}={value}"));
        }
    }
    Ok(Some(format!(
        "\n\x1b[92mRequest to litellm:\x1b[0m\n\x1b[92mlitellm.{original_function}({})\x1b[0m\n",
        rendered.join(", ")
    )))
}

// LiteLLM already printed the pair for the async entry point, so the inner sync function stays quiet
fn inner_async_call(original_function: &str, request: &RequestMap) -> bool {
    let pairs = [
        ("acompletion", "completion"),
        ("aembedding", "embedding"),
        ("aimg_generation", "img_generation"),
    ];
    pairs.iter().any(|(flag, name)| {
        *name == original_function && matches!(request.get(*flag), Some(Json::Bool(true)))
    })
}
