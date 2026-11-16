use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_media_dispatch::{
    MediaMethod, MediaPart, MediaPartBody, MediaPayload,
};
use serde::Serialize;
use serde_json::Value;

use super::failure;

// The reply fields the Python ImageResponse dump and the Rust leaf both declare, and nothing else
pub(super) fn image_reply(reply: &ImageGenerationReply) -> Value {
    let data: Vec<Value> = reply
        .data
        .iter()
        .map(|datum| serde_json::json!({"b64_json": datum.b64_json, "url": datum.url}))
        .collect();
    serde_json::json!({
        "created": reply.created,
        "background": reply.background,
        "quality": reply.quality,
        "output_format": reply.output_format,
        "size": reply.size,
        "data": data,
    })
}

pub(super) fn method_name(method: MediaMethod) -> &'static str {
    match method {
        MediaMethod::Get => "GET",
        MediaMethod::Post => "POST",
        MediaMethod::Delete => "DELETE",
    }
}

pub(super) fn serialized<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| failure("Serialize", error))
}

pub(super) fn reply_bytes(reply: &Value) -> Result<Vec<u8>, Box<HelperError>> {
    serde_json::to_vec(reply).map_err(|error| failure("Serialize", error))
}

pub(super) fn payload_value(payload: &MediaPayload) -> Result<Value, Box<HelperError>> {
    match payload {
        MediaPayload::Empty => Ok(Value::Null),
        MediaPayload::Options(fields) => Ok(Value::Object(fields.clone())),
        MediaPayload::ImageData(fields) | MediaPayload::Body(fields) => {
            serde_json::to_value(fields).map_err(|error| failure("Serialize", error))
        }
    }
}

pub(super) fn parts_value(parts: &[MediaPart]) -> Result<Value, Box<HelperError>> {
    let mut rendered = Vec::with_capacity(parts.len());
    for part in parts {
        rendered.push(serde_json::json!({
            "field": part.field,
            "filename": part.filename,
            "content_type": part.content_type,
            "body": part_body(&part.body)?,
        }));
    }
    Ok(Value::Array(rendered))
}

fn part_body(body: &MediaPartBody) -> Result<String, Box<HelperError>> {
    match body {
        MediaPartBody::Bytes(bytes) => text(bytes),
        MediaPartBody::Text(text) => Ok(text.clone()),
        // the composition never copies a caller-held reader, so no case can project one
        MediaPartBody::Streamed => Err(failure("Fixture", "the part is a stream")),
    }
}

pub(super) fn text(bytes: &[u8]) -> Result<String, Box<HelperError>> {
    String::from_utf8(bytes.to_vec()).map_err(|error| failure("Utf8", error))
}

pub(super) fn query_value(query: &[(String, String)]) -> Value {
    let mut fields = serde_json::Map::new();
    for (name, value) in query {
        fields.insert(name.clone(), Value::String(value.clone()));
    }
    Value::Object(fields)
}
