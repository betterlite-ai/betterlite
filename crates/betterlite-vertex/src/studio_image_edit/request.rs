use std::io::SeekFrom;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::StudioImageEditError;
use crate::gemini_image_policy::generation::image_generation_config;
use crate::vertex_gemini_edit::request::GeminiEditFile;

// GeminiImageEditConfig.use_multipart_form_data: an edit goes out as a JSON body with no files
pub const MULTIPART_FORM_DATA: bool = false;

// a call's image field: LiteLLM takes one record, or a list whose None members carry no image
pub enum EditImages<'image> {
    One(GeminiEditFile<'image>),
    Many(Vec<Option<GeminiEditFile<'image>>>),
}

// transform_image_edit_request: every image in order, then the prompt, and B1334's generation config
pub fn transform_image_edit_request(
    model: &str,
    prompt: Option<&str>,
    image: Option<&mut EditImages<'_>>,
    optional_params: &Object,
    mime_of: &dyn Fn(&[u8]) -> &'static str,
) -> Result<(Object, ImageFiles), StudioImageEditError> {
    let mut parts = prepare_parts(image, mime_of)?;
    if parts.is_empty() {
        return Err(StudioImageEditError::MissingImage);
    }
    // LiteLLM appends the prompt only when it is present and not empty, after the images
    if let Some(text) = prompt.filter(|text| !text.is_empty()) {
        parts.push(Json::Object(Object::from([(
            "text".to_owned(),
            Json::Text(text.to_owned()),
        )])));
    }
    let turn = Object::from([("parts".to_owned(), Json::List(parts))]);
    let body = Object::from([
        // contents is an array holding one turn, and this adapter writes no role on it
        ("contents".to_owned(), Json::List(vec![Json::Object(turn)])),
        (
            "generationConfig".to_owned(),
            Json::Object(image_generation_config(model, optional_params, None)),
        ),
    ]);
    Ok((body, ImageFiles::new()))
}

// _prepare_inline_image_parts: the records an edit can read, in the order the caller gave them
fn prepare_parts(
    image: Option<&mut EditImages<'_>>,
    mime_of: &dyn Fn(&[u8]) -> &'static str,
) -> Result<Vec<Json>, StudioImageEditError> {
    let Some(given) = image else {
        return Ok(Vec::new());
    };
    let records = match given {
        // LiteLLM tests a single image for truthiness, so empty bytes carry no image at all
        EditImages::One(GeminiEditFile::Bytes([])) => Vec::new(),
        EditImages::One(record) => vec![record],
        // only a list's None members are skipped, while an empty member still becomes a part
        EditImages::Many(items) => items.iter_mut().flatten().collect(),
    };
    let mut parts = Vec::new();
    for record in records {
        let content = image_bytes(record)?;
        parts.push(image_part(&content, mime_of));
    }
    Ok(parts)
}

// one image part: the MIME the injected reader answers, then the bytes LiteLLM base64-encodes
fn image_part(content: &[u8], mime_of: &dyn Fn(&[u8]) -> &'static str) -> Json {
    let inline_data = Object::from([
        (
            "mimeType".to_owned(),
            Json::Text(mime_of(content).to_owned()),
        ),
        ("data".to_owned(), Json::Text(STANDARD.encode(content))),
    ]);
    let mut part = Object::new();
    part.insert("inlineData".to_owned(), Json::Object(inline_data));
    Json::Object(part)
}

// _read_all_bytes: a bytes record is used as it came, and a stream is read whole from zero
fn image_bytes(image: &mut GeminiEditFile<'_>) -> Result<Vec<u8>, StudioImageEditError> {
    match image {
        GeminiEditFile::Bytes(content) => Ok(content.to_vec()),
        GeminiEditFile::Seekable(stream) => {
            let start = stream
                .stream_position()
                .map_err(StudioImageEditError::Stream)?;
            stream
                .seek(SeekFrom::Start(0))
                .map_err(StudioImageEditError::Stream)?;
            let mut content = Vec::new();
            stream
                .read_to_end(&mut content)
                .map_err(StudioImageEditError::Stream)?;
            stream
                .seek(SeekFrom::Start(start))
                .map_err(StudioImageEditError::Stream)?;
            Ok(content)
        }
        GeminiEditFile::Unsupported => Err(StudioImageEditError::UnsupportedImage),
    }
}
