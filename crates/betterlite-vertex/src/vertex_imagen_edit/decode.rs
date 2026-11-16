use betterlite_models::decisions::Json;
use betterlite_prompts::media::data_url::MAX_MEDIA_BYTES;
use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;

use super::error::ImagenEditError;
use super::input::{ImagenFields, ImagenFileInput};

// base64.b64decode accepts junk between symbols and missing padding, and the streaming decoder keeps that reading
pub fn decode_imagen_edit_input(text: &str) -> Result<Vec<u8>, ImagenEditError> {
    if text.len() > MAX_MEDIA_BYTES {
        return Err(ImagenEditError::TooLarge { size: text.len() });
    }
    // a2b_base64 stops at the first pad run that completes a quad and ignores the rest of the text
    let mut end = text.len();
    if text.is_ascii() {
        let mut quad_position = 0_usize;
        let mut pads = 0_usize;
        for (index, byte) in text.bytes().enumerate() {
            match byte {
                b'=' if quad_position >= 2 => {
                    pads = pads.saturating_add(1);
                    if quad_position.saturating_add(pads) >= 4 {
                        end = index.saturating_add(1);
                        break;
                    }
                }
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                    pads = 0;
                    quad_position = quad_position.saturating_add(1) & 3;
                }
                _ => {}
            }
        }
    }
    let read = text.get(..end).unwrap_or(text);
    decode_stream_audio_fragment(read, MAX_MEDIA_BYTES).map_err(ImagenEditError::Base64)
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
