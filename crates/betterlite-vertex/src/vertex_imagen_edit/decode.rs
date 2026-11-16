use betterlite_models::decisions::Json;
use betterlite_prompts::media::data_url::MAX_MEDIA_BYTES;
use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;

use super::error::ImagenEditError;
use super::input::{ImagenFields, ImagenFileInput};

// base64.b64decode keeps the data around a mid-string pad run and only needs a complete final quad
pub fn decode_imagen_edit_input(text: &str) -> Result<Vec<u8>, ImagenEditError> {
    if text.len() > MAX_MEDIA_BYTES {
        return Err(ImagenEditError::TooLarge { size: text.len() });
    }
    decode_stream_audio_fragment(text, MAX_MEDIA_BYTES).map_err(ImagenEditError::Base64)
}

pub fn parse_imagen_edit_input(
    body: &[u8],
) -> Result<Option<ImagenFileInput<'static>>, ImagenEditError> {
    if body.len() > MAX_MEDIA_BYTES {
        return Err(ImagenEditError::TooLarge { size: body.len() });
    }
    serde_json::from_slice(body)
        .map_err(ImagenEditError::InputJson)
        .map(project_imagen_edit_input)
}

// a JSON document is the dict, list or str record LiteLLM's reader sees, and null is no image at all
pub fn project_imagen_edit_input(document: Json) -> Option<ImagenFileInput<'static>> {
    match document {
        Json::Null => None,
        Json::Bool(_) => Some(ImagenFileInput::Unsupported { kind: "a boolean" }),
        Json::Number(_) => Some(ImagenFileInput::Unsupported { kind: "a number" }),
        Json::Text(text) => Some(ImagenFileInput::Text(text)),
        Json::List(items) => Some(ImagenFileInput::Sequence(
            items.into_iter().map(project_imagen_edit_input).collect(),
        )),
        Json::Object(mut fields) => Some(ImagenFileInput::Mapping(ImagenFields {
            data: fields
                .swap_remove("data")
                .and_then(project_imagen_edit_input)
                .map(Box::new),
            bytes: fields
                .swap_remove("bytes")
                .and_then(project_imagen_edit_input)
                .map(Box::new),
            content: fields
                .swap_remove("content")
                .and_then(project_imagen_edit_input)
                .map(Box::new),
            path: fields
                .swap_remove("path")
                .and_then(project_imagen_edit_input)
                .map(Box::new),
        })),
    }
}
