use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;
use thiserror::Error;

pub type SdkObject = IndexMap<String, SdkValue>;

// LiteLLM runs this over provider payload dicts before json.dumps, which a typed request only needs for bytes and datetimes
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SdkValue {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    Bytes(Vec<u8>),
    DateTime(SdkDateTime),
    List(Vec<Self>),
    Object(SdkObject),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SdkDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub microsecond: u32,
    pub offset_seconds: Option<i32>,
}

impl SdkDateTime {
    #[must_use]
    pub fn isoformat(&self) -> String {
        let date = format!("{:04}-{:02}-{:02}", self.year, self.month, self.day);
        let time = format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second);
        let fraction = if self.microsecond == 0 {
            String::new()
        } else {
            format!(".{:06}", self.microsecond)
        };
        let offset = self.offset_seconds.map_or_else(String::new, offset_text);
        format!("{date}T{time}{fraction}{offset}")
    }
}

fn offset_text(offset: i32) -> String {
    let magnitude = offset.unsigned_abs();
    let hours = magnitude.saturating_div(3_600);
    let minutes = magnitude
        .saturating_sub(hours.saturating_mul(3_600))
        .saturating_div(60);
    let seconds = magnitude
        .saturating_sub(hours.saturating_mul(3_600))
        .saturating_sub(minutes.saturating_mul(60));
    let sign = if offset < 0_i32 { '-' } else { '+' };
    let tail = if seconds == 0 {
        String::new()
    } else {
        format!(":{seconds:02}")
    };
    format!("{sign}{hours:02}:{minutes:02}{tail}")
}

#[derive(Debug, Error)]
pub enum SdkError {
    #[error("an SDK value holds bytes or a datetime, which JSON cannot serialize")]
    UnsupportedValue,
}

#[must_use]
pub fn encode_unserializable_types(values: SdkObject, depth: i64, max_depth: i64) -> SdkObject {
    if depth > max_depth {
        return values;
    }
    let mut processed = SdkObject::new();
    for (key, value) in values {
        let encoded = match value {
            SdkValue::Bytes(bytes) => SdkValue::Text(URL_SAFE.encode(bytes)),
            SdkValue::DateTime(moment) => SdkValue::Text(moment.isoformat()),
            SdkValue::Object(nested) => SdkValue::Object(encode_unserializable_types(
                nested,
                depth.saturating_add(1),
                max_depth,
            )),
            SdkValue::List(items) => {
                SdkValue::List(encode_list(items, depth.saturating_add(1), max_depth))
            }
            other @ (SdkValue::Null
            | SdkValue::Bool(_)
            | SdkValue::Number(_)
            | SdkValue::Text(_)) => other,
        };
        processed.insert(key, encoded);
    }
    processed
}

fn encode_list(items: Vec<SdkValue>, depth: i64, max_depth: i64) -> Vec<SdkValue> {
    // the source's two independent tests overwrite the all-bytes conversion, so only the datetime branch survives
    if items
        .iter()
        .all(|item| matches!(item, SdkValue::DateTime(_)))
    {
        return items
            .into_iter()
            .map(|item| match item {
                SdkValue::DateTime(moment) => SdkValue::Text(moment.isoformat()),
                other @ (SdkValue::Null
                | SdkValue::Bool(_)
                | SdkValue::Number(_)
                | SdkValue::Text(_)
                | SdkValue::Bytes(_)
                | SdkValue::List(_)
                | SdkValue::Object(_)) => other,
            })
            .collect();
    }
    // every other list item goes to the source's dict-only encoder, so bytes and nested lists stay as they are
    items
        .into_iter()
        .map(|item| match item {
            SdkValue::Object(fields) => {
                SdkValue::Object(encode_unserializable_types(fields, depth, max_depth))
            }
            other @ (SdkValue::Null
            | SdkValue::Bool(_)
            | SdkValue::Number(_)
            | SdkValue::Text(_)
            | SdkValue::Bytes(_)
            | SdkValue::DateTime(_)
            | SdkValue::List(_)) => other,
        })
        .collect()
}

pub fn to_object(values: &SdkObject) -> Result<Object, SdkError> {
    let mut object = Object::new();
    for (key, value) in values {
        object.insert(key.clone(), to_json(value)?);
    }
    Ok(object)
}

fn to_json(value: &SdkValue) -> Result<Json, SdkError> {
    Ok(match value {
        SdkValue::Null => Json::Null,
        SdkValue::Bool(flag) => Json::Bool(*flag),
        SdkValue::Number(number) => Json::Number(number.clone()),
        SdkValue::Text(text) => Json::Text(text.clone()),
        SdkValue::Bytes(_) | SdkValue::DateTime(_) => return Err(SdkError::UnsupportedValue),
        SdkValue::List(items) => {
            Json::List(items.iter().map(to_json).collect::<Result<Vec<_>, _>>()?)
        }
        SdkValue::Object(fields) => Json::Object(to_object(fields)?),
    })
}
