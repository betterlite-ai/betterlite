use std::collections::BTreeMap;
use std::str::from_utf8;

use serde::Deserialize;
use serde::de::IgnoredAny;

use super::error::EventFrameError;

// The value types botocore tells apart for a chunk field: null is absent, text is base64, anything else is wrong.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BodyField {
    Null(()),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Body {
    Object(BTreeMap<String, BodyField>),
    NotObject(IgnoredAny),
}

// botocore's EventStreamJSONParser body read: empty is {}, and text that is not JSON becomes {"message": body}, a key no caller reads.
pub fn parse_body_object(payload: &[u8]) -> Result<BTreeMap<String, BodyField>, EventFrameError> {
    if payload.is_empty() {
        return Ok(BTreeMap::new());
    }
    let body = from_utf8(payload).map_err(EventFrameError::NotUtf8)?;
    match serde_json::from_str::<Body>(body) {
        Ok(Body::Object(object)) => Ok(object),
        Ok(Body::NotObject(_)) => Err(EventFrameError::PayloadNotObject),
        Err(_) => Ok(BTreeMap::new()),
    }
}
