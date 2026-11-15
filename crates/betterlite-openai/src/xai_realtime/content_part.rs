use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::XaiRealtimeError;

pub const MAX_CACHED_CONTENT_PARTS: usize = 4096;

const CONTENT_PART_ADDED: &str = "response.content_part.added";
const CONTENT_PART_DONE: &str = "response.content_part.done";
const TEXT_DONE: &str = "response.output_text.done";
const TRANSCRIPT_DONE: &str = "response.output_audio_transcript.done";

// LiteLLM keys the cache on the event's own response_id, item_id and content_index values
#[derive(Clone, Debug, PartialEq, Eq)]
struct ContentPartKey {
    response_id: Option<Json>,
    item_id: Option<Json>,
    content_index: Json,
}

impl Hash for ContentPartKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_part(self.response_id.as_ref(), state);
        hash_part(self.item_id.as_ref(), state);
        hash_part(Some(&self.content_index), state);
    }
}

fn hash_part<H: Hasher>(value: Option<&Json>, state: &mut H) {
    match value {
        None => state.write_u8(0),
        Some(Json::Null) => state.write_u8(1),
        Some(Json::Bool(flag)) => {
            state.write_u8(2);
            flag.hash(state);
        }
        Some(Json::Number(number)) => {
            state.write_u8(3);
            number.hash(state);
        }
        Some(Json::Text(text)) => {
            state.write_u8(4);
            text.hash(state);
        }
        Some(Json::List(items)) => {
            state.write_u8(5);
            items.len().hash(state);
            for item in items {
                hash_part(Some(item), state);
            }
        }
        Some(Json::Object(entries)) => {
            state.write_u8(6);
            entries.len().hash(state);
            // IndexMap equality ignores entry order, so the hash sorts the entries to match it
            let mut ordered: Vec<(&String, &Json)> = entries.iter().collect();
            ordered.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (name, entry) in ordered {
                name.hash(state);
                hash_part(Some(entry), state);
            }
        }
    }
}

fn key_of(fields: &IndexMap<String, Json>) -> ContentPartKey {
    ContentPartKey {
        response_id: non_null(fields.get("response_id")),
        item_id: non_null(fields.get("item_id")),
        content_index: fields
            .get("content_index")
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_u8))),
    }
}

fn non_null(value: Option<&Json>) -> Option<Json> {
    match value {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

#[derive(Debug, Default)]
pub struct ContentPartCache {
    parts: HashMap<ContentPartKey, Json>,
}

impl ContentPartCache {
    fn remember(&mut self, fields: &IndexMap<String, Json>) -> Result<(), XaiRealtimeError> {
        let Some(part) = fields.get("part") else {
            return Ok(());
        };
        if !matches!(part, Json::Object(_)) {
            return Ok(());
        }
        self.store(key_of(fields), part.clone())
    }

    fn update(
        &mut self,
        fields: &IndexMap<String, Json>,
        part_type: &str,
        field: &str,
    ) -> Result<(), XaiRealtimeError> {
        let Some(value) = fields.get(field) else {
            return Ok(());
        };
        if matches!(value, Json::Null) {
            return Ok(());
        }
        let key = key_of(fields);
        let updated = match self.parts.get(&key) {
            Some(Json::Object(entries)) => {
                let mut entries = entries.clone();
                entries
                    .entry(String::from("type"))
                    .or_insert_with(|| Json::Text(part_type.to_owned()));
                entries.insert(field.to_owned(), value.clone());
                Json::Object(entries)
            }
            _ => Json::Object(IndexMap::from([
                (String::from("type"), Json::Text(part_type.to_owned())),
                (field.to_owned(), value.clone()),
            ])),
        };
        self.store(key, updated)
    }

    fn resolve(&self, fields: &IndexMap<String, Json>) -> Json {
        if let Some(part) = fields.get("part")
            && matches!(part, Json::Object(_))
        {
            return part.clone();
        }
        if let Some(part) = self.parts.get(&key_of(fields)) {
            return part.clone();
        }
        Json::Object(IndexMap::from([
            (String::from("type"), Json::Text(String::from("audio"))),
            (String::from("transcript"), Json::Text(String::new())),
        ]))
    }

    fn store(&mut self, key: ContentPartKey, part: Json) -> Result<(), XaiRealtimeError> {
        if !self.parts.contains_key(&key) && self.parts.len() >= MAX_CACHED_CONTENT_PARTS {
            return Err(XaiRealtimeError::ContentPartLimit {
                limit: MAX_CACHED_CONTENT_PARTS,
            });
        }
        self.parts.insert(key, part);
        Ok(())
    }
}

pub fn normalize_event(
    event: Json,
    event_type: Option<&str>,
    cache: &mut ContentPartCache,
) -> Result<Json, XaiRealtimeError> {
    match event {
        Json::Object(mut fields) => {
            match event_type {
                Some(CONTENT_PART_ADDED | CONTENT_PART_DONE) => {
                    cache.remember(&fields)?;
                    if !matches!(fields.get("part"), Some(Json::Object(_))) {
                        let part = cache.resolve(&fields);
                        fields.insert(String::from("part"), part);
                    }
                }
                Some(TEXT_DONE) => cache.update(&fields, "text", "text")?,
                Some(TRANSCRIPT_DONE) => cache.update(&fields, "audio", "transcript")?,
                Some(_) | None => {}
            }
            Ok(Json::Object(fields))
        }
        other => Ok(other),
    }
}
