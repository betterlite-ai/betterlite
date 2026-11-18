use std::borrow::Cow;
use std::collections::BTreeMap;
use std::str;

use thiserror::Error;

mod parser;

pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
pub const MAX_MESSAGE_DEPTH: usize = 64;

pub type TypedValue = BTreeMap<String, JsonValue>;

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<JsonValue>),
    Object(TypedValue),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("the invalidation message is {length} bytes, over the {limit} byte limit")]
    TooLarge { length: usize, limit: usize },
    #[error("the invalidation message nests deeper than {limit} containers")]
    TooDeep { limit: usize },
    #[error("the invalidation message is not valid JSON")]
    Syntax,
    #[error("the invalidation message is not a JSON object")]
    NotAnObject,
}

pub fn decode_invalidation(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(DecodeError::TooLarge {
            length: bytes.len(),
            limit: MAX_MESSAGE_BYTES,
        });
    }
    let text = repaired_text(bytes);
    parser::parse_object(&text)
}

fn repaired_text(bytes: &[u8]) -> Cow<'_, str> {
    match str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text),
        Err(error) => {
            let mut repaired = String::with_capacity(bytes.len());
            for chunk in error.utf8_chunks() {
                repaired.push_str(chunk.valid());
                if !chunk.invalid().is_empty() {
                    repaired.push('\u{fffd}');
                }
            }
            Cow::Owned(repaired)
        }
    }
}
