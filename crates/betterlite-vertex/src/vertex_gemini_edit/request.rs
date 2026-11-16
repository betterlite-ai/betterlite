use std::io::{Read, Seek, SeekFrom};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};

use super::error::GeminiEditError;

// the typed file records an edit request can carry, as LiteLLM's reader sees them
pub enum GeminiEditFile<'image> {
    // a bytes record, used as it came
    Bytes(&'image [u8]),
    // a BytesIO or BufferedReader equivalent, read from position zero and left where it was
    Seekable(&'image mut (dyn Read + Seek)),
    // every other record, which LiteLLM's reader raises ValueError for
    Unsupported,
}

// transform_image_edit_request: the JSON generateContent body and no multipart files
pub fn transform_image_edit_request(
    prompt: Option<&str>,
    images: &mut [Option<GeminiEditFile<'_>>],
    optional_params: &ImageParams,
    mime_of: &dyn Fn(&[u8]) -> &'static str,
) -> Result<(ImageParams, ImageFiles), GeminiEditError> {
    let mut parts = inline_parts(images, mime_of)?;
    if parts.is_empty() {
        return Err(GeminiEditError::MissingImage);
    }
    // LiteLLM appends the prompt only when it is neither absent nor empty, after the images
    if let Some(prompt) = prompt.filter(|text| !text.is_empty()) {
        let mut part = ImageParams::new();
        part.insert("text".to_owned(), Json::Text(prompt.to_owned()));
        parts.push(Json::Object(part));
    }
    let mut contents = ImageParams::new();
    contents.insert("role".to_owned(), Json::Text("USER".to_owned()));
    contents.insert("parts".to_owned(), Json::List(parts));
    let mut generation_config = ImageParams::new();
    generation_config.insert(
        "response_modalities".to_owned(),
        Json::List(vec![Json::Text("IMAGE".to_owned())]),
    );
    // LiteLLM adds image_config only when the mapped params hold an aspect ratio
    if let Some(ratio) = optional_params.get("aspectRatio") {
        let mut image_config = ImageParams::new();
        image_config.insert("aspect_ratio".to_owned(), ratio.clone());
        generation_config.insert("image_config".to_owned(), Json::Object(image_config));
    }
    let mut body = ImageParams::new();
    body.insert("contents".to_owned(), Json::Object(contents));
    body.insert(
        "generationConfig".to_owned(),
        Json::Object(generation_config),
    );
    Ok((body, ImageFiles::new()))
}

// a None entry carries no image, and every other record becomes one part in list order
fn inline_parts(
    images: &mut [Option<GeminiEditFile<'_>>],
    mime_of: &dyn Fn(&[u8]) -> &'static str,
) -> Result<Vec<Json>, GeminiEditError> {
    let mut parts = Vec::with_capacity(images.len());
    for image in images.iter_mut().flatten() {
        parts.push(inline_part(image, mime_of)?);
    }
    Ok(parts)
}

fn inline_part(
    image: &mut GeminiEditFile<'_>,
    mime_of: &dyn Fn(&[u8]) -> &'static str,
) -> Result<Json, GeminiEditError> {
    let content = image_bytes(image)?;
    let mut inline = ImageParams::new();
    inline.insert(
        "mimeType".to_owned(),
        Json::Text(mime_of(&content).to_owned()),
    );
    inline.insert("data".to_owned(), Json::Text(STANDARD.encode(&content)));
    let mut part = ImageParams::new();
    part.insert("inlineData".to_owned(), Json::Object(inline));
    Ok(Json::Object(part))
}

// _read_all_bytes: a bytes record is used as it came and a seekable stream is read whole from zero
fn image_bytes(image: &mut GeminiEditFile<'_>) -> Result<Vec<u8>, GeminiEditError> {
    match image {
        GeminiEditFile::Bytes(content) => Ok(content.to_vec()),
        GeminiEditFile::Seekable(stream) => read_seekable(&mut **stream),
        GeminiEditFile::Unsupported => Err(GeminiEditError::UnsupportedImage),
    }
}

// LiteLLM rewinds a BytesIO or BufferedReader, reads it and puts the cursor back
fn read_seekable(stream: &mut (dyn Read + Seek)) -> Result<Vec<u8>, GeminiEditError> {
    let start = stream.stream_position().map_err(GeminiEditError::Stream)?;
    stream
        .seek(SeekFrom::Start(0))
        .map_err(GeminiEditError::Stream)?;
    let mut content = Vec::new();
    stream
        .read_to_end(&mut content)
        .map_err(GeminiEditError::Stream)?;
    stream
        .seek(SeekFrom::Start(start))
        .map_err(GeminiEditError::Stream)?;
    Ok(content)
}
