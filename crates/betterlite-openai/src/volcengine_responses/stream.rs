use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_event_records::records::{GenericEvent, KnownResponseEvent};
use crate::api::responses_shapes::wire::{WireDecodeLimits, decode_responses_shapes};
use crate::response_events::event_decode::{DecodedStreamResponse, EventClass};

use super::error::VolcengineResponsesError;
use super::fields::fill_event;

const DECODE_LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0010_0000,
    max_items: 0x4000,
};

// the outside-byte boundary: an event payload arrives as bytes and leaves as a validated record
pub fn decode_stream_event(
    bytes: &[u8],
) -> Result<DecodedStreamResponse, VolcengineResponsesError> {
    let decoded: Json = decode_responses_shapes(bytes, DECODE_LIMITS)
        .map_err(|source| VolcengineResponsesError::EventDecode { source })?;
    let Json::Object(fields) = decoded else {
        return Err(VolcengineResponsesError::EventNotAnObject);
    };
    transform_streaming_response(&fields)
}

// LiteLLM fills a response without output first, then every missing declared field of the event's record
pub fn transform_streaming_response(
    chunk: &IndexMap<String, Json>,
) -> Result<DecodedStreamResponse, VolcengineResponsesError> {
    let mut patched = chunk.clone();
    if let Some(Json::Object(response)) = patched.get_mut("response") {
        response
            .entry(String::from("output"))
            .or_insert_with(|| Json::List(Vec::new()));
    }
    let Some(Json::Text(event_type)) = patched.get("type") else {
        return Err(VolcengineResponsesError::EventType);
    };
    let class = EventClass::of(event_type);
    fill_event(class, &mut patched);
    decode_event(class, patched)
}

fn decode_event(
    class: EventClass,
    patched: IndexMap<String, Json>,
) -> Result<DecodedStreamResponse, VolcengineResponsesError> {
    let value = serde_json::to_value(Json::Object(patched))
        .map_err(|source| VolcengineResponsesError::EventEncode { source })?;
    match class {
        EventClass::Known(_known) => {
            let record: KnownResponseEvent = serde_json::from_value(value)
                .map_err(|source| VolcengineResponsesError::EventRecord { source })?;
            Ok(DecodedStreamResponse::Known {
                class,
                record: Box::new(record),
            })
        }
        EventClass::Generic => {
            let event: GenericEvent = serde_json::from_value(value)
                .map_err(|source| VolcengineResponsesError::EventRecord { source })?;
            Ok(DecodedStreamResponse::Generic(event))
        }
    }
}
