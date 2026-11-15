use std::str::{Utf8Error, from_utf8};

use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde_json::Deserializer;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LineError {
    #[error("a batch line is not UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("a batch line must be a JSON object")]
    NotAnObject,
    #[error("the batch line value at `{path}` is not valid")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

// serde_json's own object type sorts keys, and the row builders read these fields in the order the caller sent them
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct BatchLine {
    pub custom_id: Option<String>,
    pub url: Option<Json>,
    pub body: Option<Json>,
}

// a blank line is no entry, and a line that is not JSON fails the whole upload as json.loads does
pub fn decode_openai_batch_line(line: &[u8]) -> Result<Option<BatchLine>, LineError> {
    let text = from_utf8(line).map_err(LineError::Utf8)?.trim();
    if text.is_empty() {
        return Ok(None);
    }
    // serde would read a JSON array as the struct's fields in order, but LiteLLM only accepts an object
    if text.starts_with('[') {
        return Err(LineError::NotAnObject);
    }
    let mut decoder = Deserializer::from_str(text);
    let entry =
        serde_path_to_error::deserialize(&mut decoder).map_err(|error| LineError::Json {
            path: error.path().to_string(),
            source: error.into_inner(),
        })?;
    decoder.end().map_err(|source| LineError::Json {
        path: String::from("."),
        source,
    })?;
    Ok(Some(entry))
}
