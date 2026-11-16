use betterlite_models::decisions::{Extra, Json};
use betterlite_prompts::media::data_url::MAX_MEDIA_BYTES;
use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;

use super::error::ImagenEditError;
use super::input::{ImagenFields, ImagenFileInput};

// base64.b64decode accepts junk between symbols and missing padding, and the streaming decoder keeps that reading
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
        Json::Object(fields) => Some(ImagenFileInput::Mapping(ImagenFields {
            data: named_field(&fields, "data"),
            bytes: named_field(&fields, "bytes"),
            content: named_field(&fields, "content"),
            path: named_field(&fields, "path"),
        })),
    }
}

fn named_field(fields: &Extra, name: &str) -> Option<Box<ImagenFileInput<'static>>> {
    let value = fields.get(name)?;
    let image = project_imagen_edit_input(value.clone())?;
    Some(Box::new(image))
}
