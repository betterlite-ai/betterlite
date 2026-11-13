use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_tokenizer::image::header::image_type;

use crate::api::file_type_records::mime_from_type;
use crate::api::image_contracts::records::{ImageFile, ImageFiles};

pub const DEFAULT_IMAGE_NAME: &str = "image.png";
pub const DEFAULT_MASK_NAME: &str = "mask.png";
pub const DEFAULT_CONTENT_TYPE: &str = "image/png";
// get_image_content_type reads at most the first 100 bytes, and no signature needs more than 12
const CONTENT_TYPE_BYTES: usize = 100;

// LiteLLM's FileTypes: only an open BufferedReader carries a name, so every other value uploads as image.png
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageEditFile {
    pub content: Vec<u8>,
    pub name: Option<String>,
}

#[must_use]
pub fn file_record(field: &str, file: &ImageEditFile, default_name: &str) -> (String, ImageFile) {
    let head = file
        .content
        .get(..CONTENT_TYPE_BYTES)
        .unwrap_or(&file.content);
    let content_type = image_type(head).map_or(DEFAULT_CONTENT_TYPE, mime_from_type);
    (
        field.to_owned(),
        ImageFile {
            filename: Some(file.name.clone().unwrap_or_else(|| default_name.to_owned())),
            content: file.content.clone(),
            content_type: Some(content_type.to_owned()),
        },
    )
}

// the httpx files argument as the multipart body betterlite-http sends, in the order LiteLLM builds it
#[must_use]
pub fn multipart_parts(files: &ImageFiles) -> Vec<MultipartPart> {
    files
        .iter()
        .map(|(field, file)| MultipartPart {
            name: field.clone(),
            file_name: file.filename.clone(),
            content_type: file.content_type.clone(),
            data: file.content.clone(),
        })
        .collect()
}
