#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use betterlite_prompts::arguments::json::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::databricks_chat::response::{ResponseInput, transform_response};
use crate::databricks_chat::stream::ChunkParser;
use crate::ollama_chat::ids::RandomIds;

// JSON for the tests as the typed Json enum: interpolation goes through object() and list()
macro_rules! json {
    ($($part:tt)*) => {
        serde_json::from_str::<Json>(stringify!($($part)*)).unwrap()
    };
}
pub(crate) use json;

static NULL: Json = Json::Null;

// The value at a dotted path of object keys and list positions, or null when the path leads nowhere
pub(crate) fn at<'doc>(doc: &'doc Json, path: &str) -> &'doc Json {
    let mut current = doc;
    for step in path.split('.') {
        current = match current {
            Json::Object(fields) => fields.get(step).unwrap_or(&NULL),
            Json::List(items) => step
                .parse::<usize>()
                .ok()
                .and_then(|position| items.get(position))
                .unwrap_or(&NULL),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => &NULL,
        };
    }
    current
}

// The value at a dotted path for a test that edits the document, which must exist
pub(crate) fn at_mut<'doc>(doc: &'doc mut Json, path: &str) -> &'doc mut Json {
    let mut current = doc;
    for step in path.split('.') {
        current = match current {
            Json::Object(fields) => fields.get_mut(step).unwrap(),
            Json::List(items) => items.get_mut(step.parse::<usize>().unwrap()).unwrap(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                panic!("the path {path} leaves the document at {step}")
            }
        };
    }
    current
}

// Sets one key of an object document
pub(crate) fn set(doc: &mut Json, key: &str, value: Json) {
    if let Json::Object(fields) = doc {
        fields.insert(key.to_owned(), value);
    }
}

// Removes one key of an object document
pub(crate) fn unset(doc: &mut Json, key: &str) {
    if let Json::Object(fields) = doc {
        fields.shift_remove(key);
    }
}

// The length of a list document, or None for any other document
pub(crate) fn len_of(doc: &Json) -> Option<usize> {
    match doc {
        Json::List(items) => Some(items.len()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

// Whether the object holds the key at all, which at() cannot tell from a null value
pub(crate) fn has(doc: &Json, key: &str) -> bool {
    doc.as_object()
        .is_some_and(|fields| fields.contains_key(key))
}

// An object built from values the test already holds
pub(crate) fn object<const FIELDS: usize>(fields: [(&str, Json); FIELDS]) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

// The document with every text equal to the placeholder replaced by the serialized value
pub(crate) fn fill<Source: Serialize + ?Sized>(
    doc: Json,
    placeholder: &str,
    with: &Source,
) -> Json {
    match doc {
        Json::Text(text) if text == placeholder => dump(with),
        Json::List(items) => Json::List(
            items
                .into_iter()
                .map(|item| fill(item, placeholder, with))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name, fill(value, placeholder, with)))
                .collect(),
        ),
        leaf @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => leaf,
    }
}

// The compact JSON text of a document, as the wire carries it
pub(crate) fn text_of(doc: &Json) -> String {
    serde_json::to_string(doc).unwrap()
}

// Any serializable value as the Json the wire would carry
pub(crate) fn dump<Source: Serialize>(value: &Source) -> Json {
    convert(value)
}

// One typed shape read from another through JSON
pub(crate) fn convert<Source: Serialize + ?Sized, Target: DeserializeOwned>(
    value: &Source,
) -> Target {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

// Every corpus file goes through the reply decoder and the chunk parser, in and out of json mode
#[test]
fn replays_the_databricks_chat_reply_corpus() {
    let outcomes: Vec<(bool, bool)> = fs::read_dir("fuzz/corpus/databricks_chat_reply")
        .expect("the reply corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter_map(|path| fs::read(path).ok())
        .filter_map(|bytes| String::from_utf8(bytes).ok())
        .flat_map(|text| [false, true].map(|json_mode| (text.clone(), json_mode)))
        .map(|(text, json_mode)| {
            let reply = transform_response(&ResponseInput {
                body: &text,
                status_code: 200,
                headers: BTreeMap::new(),
                custom_llm_provider: None,
                json_mode,
                ids: &RandomIds,
            });
            let chunk = ChunkParser {
                json_mode,
                last_function_name: None,
            }
            .parse(&text, &RandomIds);
            (reply.is_ok(), chunk.is_ok())
        })
        .collect();
    assert!(outcomes.iter().any(|(reply, _)| *reply));
    assert!(outcomes.iter().any(|(_, chunk)| *chunk));
}
