use betterlite_core::api::response_id_traversal::metadata::AffinityMetadata;
use betterlite_core::api::response_id_traversal::response_id::{rewrite_value, transcode};
use betterlite_core::api::response_synthetic_events::error::SyntheticEventsError;
use betterlite_core::api::response_synthetic_events::model_dump::HasModelDumpJson;
use betterlite_core::api::responses_ws_managed::provider::ProviderChunk;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, decode_response_event,
};

use super::null_shape::{Probe, Shape};
use super::{AdapterError, ScriptedFailure};
use indexmap::IndexMap;
use serde::Serialize as _;

struct BrokenChunk;

impl HasModelDumpJson for BrokenChunk {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject { field: "chunk" })
    }
}

pub(crate) fn provider_chunk(bytes: &[u8]) -> Result<Option<ProviderChunk>, AdapterError> {
    let value = match decode_response_event(bytes)? {
        DecodedStreamResponse::Known { record, .. } => {
            let shape = record.serialize(Probe)?;
            let mut encoded = transcode(&record)?;
            omit_absent(&mut encoded, &shape);
            encoded
        }
        DecodedStreamResponse::Generic(generic) => transcode(&generic)?,
        DecodedStreamResponse::Lax { fields, .. } => Json::Object(fields),
    };
    adapt_event(value)
}

// HTTP fault markers keep boundary failures testable without bypassing B927's event validation.
fn adapt_event(mut value: Json) -> Result<Option<ProviderChunk>, AdapterError> {
    if let Json::Object(fields) = &mut value {
        match fields.get("type") {
            Some(Json::Text(kind)) if kind == "fixture.empty" => {
                return Ok(Some(ProviderChunk::Empty));
            }
            Some(Json::Text(kind)) if kind == "fixture.broken" => {
                return Ok(Some(ProviderChunk::Object {
                    kind: None,
                    dump: Box::new(BrokenChunk),
                }));
            }
            Some(Json::Text(kind)) if kind == "fixture.error" => {
                if let Some(Json::Text(message)) = fields.get("message") {
                    return Err(AdapterError::Scripted(ScriptedFailure {
                        message: message.clone(),
                    }));
                }
            }
            Some(Json::Text(kind)) if kind == "fixture.end" => return Ok(None),
            Some(Json::Text(kind)) if kind == "fixture.completed" => {
                restore_completion(fields);
                return Ok(Some(ProviderChunk::Json(value)));
            }
            Some(
                Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)
                | Json::Object(_),
            )
            | None => {}
        }
    }
    rewrite_event(value)
}

fn rewrite_event(mut value: Json) -> Result<Option<ProviderChunk>, AdapterError> {
    if let Json::Object(fields) = &mut value {
        fields.retain(|_, child| !matches!(child, Json::Null));
        if let Some(response) = fields.get_mut("response") {
            rewrite_value(response, Some("openai"), &AffinityMetadata::default())?;
        }
    }
    Ok(Some(ProviderChunk::Json(value)))
}

// Keep original JSON numbers and dictionary entries; only typed model fields follow exclude_none.
fn omit_absent(value: &mut Json, shape: &Shape) {
    match shape {
        Shape::Model(fields) | Shape::Dictionary(fields) => {
            if let Json::Object(object) = value {
                object.retain(|name, child| {
                    let Some(field) = fields.get(name) else {
                        return true;
                    };
                    if matches!(shape, Shape::Model(_)) && matches!(field, Shape::Null) {
                        return false;
                    }
                    omit_absent(child, field);
                    true
                });
            }
        }
        Shape::Sequence(fields) => {
            if let Json::List(items) = value {
                for (child, field) in items.iter_mut().zip(fields) {
                    omit_absent(child, field);
                }
            }
        }
        Shape::Null | Shape::Value => {}
    }
}

fn restore_completion(fields: &mut IndexMap<String, Json>) {
    if let Some(response) = fields.shift_remove("fixture_response") {
        fields.insert(String::from("response"), response);
    }
    fields.insert(
        String::from("type"),
        Json::Text(String::from("response.completed")),
    );
}
