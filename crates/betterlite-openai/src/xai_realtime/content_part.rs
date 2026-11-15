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
    response_id: Json,
    item_id: Json,
    content_index: Json,
}

fn key_of(fields: &IndexMap<String, Json>) -> ContentPartKey {
    ContentPartKey {
        response_id: fields.get("response_id").cloned().unwrap_or(Json::Null),
        item_id: fields.get("item_id").cloned().unwrap_or(Json::Null),
        content_index: fields
            .get("content_index")
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_u8))),
    }
}

#[derive(Debug, Default)]
pub struct ContentPartCache {
    // only part events scan the cache, so the 4096 bound keeps each lookup cheap
    parts: Vec<(ContentPartKey, Json)>,
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
        let existing = self
            .parts
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, part)| part.clone());
        let updated = match existing {
            Some(Json::Object(entries)) => {
                let mut merged = entries;
                merged
                    .entry(String::from("type"))
                    .or_insert_with(|| Json::Text(part_type.to_owned()));
                merged.insert(field.to_owned(), value.clone());
                Json::Object(merged)
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
        let key = key_of(fields);
        if let Some(part) = self
            .parts
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, part)| part)
        {
            return part.clone();
        }
        Json::Object(IndexMap::from([
            (String::from("type"), Json::Text(String::from("audio"))),
            (String::from("transcript"), Json::Text(String::new())),
        ]))
    }

    fn store(&mut self, key: ContentPartKey, part: Json) -> Result<(), XaiRealtimeError> {
        let existing = self
            .parts
            .iter_mut()
            .find(|entry| entry.0 == key)
            .map(|entry| &mut entry.1);
        if let Some(slot) = existing {
            *slot = part;
            return Ok(());
        }
        if self.parts.len() >= MAX_CACHED_CONTENT_PARTS {
            return Err(XaiRealtimeError::ContentPartLimit {
                limit: MAX_CACHED_CONTENT_PARTS,
            });
        }
        self.parts.push((key, part));
        Ok(())
    }
}

pub fn normalize_event(
    mut event: Json,
    event_type: Option<&str>,
    cache: &mut ContentPartCache,
) -> Result<Json, XaiRealtimeError> {
    if let Json::Object(fields) = &mut event {
        match event_type {
            Some(CONTENT_PART_ADDED | CONTENT_PART_DONE) => {
                cache.remember(fields)?;
                if !matches!(fields.get("part"), Some(Json::Object(_))) {
                    let part = cache.resolve(fields);
                    fields.insert(String::from("part"), part);
                }
            }
            Some(TEXT_DONE) => cache.update(fields, "text", "text")?,
            Some(TRANSCRIPT_DONE) => cache.update(fields, "audio", "transcript")?,
            Some(_) | None => {}
        }
    }
    Ok(event)
}
