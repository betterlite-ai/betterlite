use betterlite_core::api::response_id_traversal::metadata::AffinityMetadata;
use betterlite_core::api::response_id_traversal::response_id::{rewrite_value, transcode};
use betterlite_core::api::response_synthetic_events::error::SyntheticEventsError;
use betterlite_core::api::response_synthetic_events::model_dump::HasModelDumpJson;
use betterlite_core::api::responses_ws_managed::provider::ProviderChunk;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, decode_response_event,
};

use super::{AdapterError, ScriptedFailure};
use indexmap::IndexMap;

struct BrokenChunk;

impl HasModelDumpJson for BrokenChunk {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject { field: "chunk" })
    }
}

pub(crate) fn provider_chunk(bytes: &[u8]) -> Result<Option<ProviderChunk>, AdapterError> {
    let value = match decode_response_event(bytes)? {
        DecodedStreamResponse::Known { record, .. } => transcode(&record)?,
        DecodedStreamResponse::Generic(generic) => transcode(&generic)?,
        DecodedStreamResponse::Lax { fields, .. } => Json::Object(fields),
    };
    adapt_event(value)
}

// Fault markers keep B945 boundary failures testable after B927 validates provider events.
// They travel in the HTTP body, so the adapter cannot pass without consuming it.
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
    if let Json::Object(fields) = &mut value
        && let Some(response) = fields.get_mut("response")
    {
        rewrite_value(response, Some("openai"), &AffinityMetadata::default())?;
    }
    Ok(Some(ProviderChunk::Json(without_nulls(value))))
}

fn without_nulls(value: Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .filter(|(_, child)| !matches!(child, Json::Null))
                .map(|(name, child)| (name, without_nulls(child)))
                .collect(),
        ),
        Json::List(items) => Json::List(items.into_iter().map(without_nulls).collect()),
        scalar @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => scalar,
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
