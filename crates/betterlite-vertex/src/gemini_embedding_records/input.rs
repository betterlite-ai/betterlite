use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::gemini_embedding_records::data_url::{
    EmbeddingDataUrl, decode_embedding_data_url, infer_mime_from_gcs_url,
};
use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_media::part::{Blob, FileData};
pub const IMAGE_MIME_TYPES: [&str; 2] = ["image/jpeg", "image/png"];
const DATA_MARKER: &str = ";base64,";
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Text(String),
    Entries(Vec<InputEntry>),
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InputEntry {
    Text(String),
    Group(Vec<String>),
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedFile {
    pub mime_type: String,
    pub uri: String,
}
pub type ResolvedFiles = BTreeMap<String, ResolvedFile>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Text(String),
    InlineData(Blob),
    FileData(FileData),
}
pub enum Element<'element> {
    Text(&'element str),
    Inline(EmbeddingDataUrl<'element>),
    Gcs(&'static str, &'element str),
    File(&'element ResolvedFile),
}
#[derive(Clone, Copy, Debug)]
pub struct InputShape {
    pub media: bool,
    pub nested: bool,
}
#[must_use]
pub fn shape(input: &EmbeddingInput) -> InputShape {
    match input {
        EmbeddingInput::Text(text) => InputShape {
            media: is_media(text),
            nested: false,
        },
        EmbeddingInput::Entries(entries) => InputShape {
            media: entries.iter().any(|entry| match entry {
                InputEntry::Text(text) => is_media(text),
                InputEntry::Group(group) => group.iter().any(|element| is_media(element)),
            }),
            nested: entries
                .iter()
                .any(|entry| matches!(entry, InputEntry::Group(_))),
        },
    }
}
#[must_use]
pub fn is_media(element: &str) -> bool {
    (element.starts_with("data:") && element.contains(DATA_MARKER))
        || element.starts_with("files/")
        || element.starts_with("gs://")
}
pub fn classify<'element>(
    element: &'element str,
    resolved_files: &'element ResolvedFiles,
) -> Result<Element<'element>, GeminiEmbeddingError> {
    if element.starts_with("data:") && element.contains(DATA_MARKER) {
        return decode_embedding_data_url(element).map(Element::Inline);
    }
    if element.starts_with("gs://") {
        return infer_mime_from_gcs_url(element).map(|mime_type| Element::Gcs(mime_type, element));
    }
    if element.starts_with("files/") {
        let Some(file) = resolved_files.get(element) else {
            return Err(GeminiEmbeddingError::Rejected {
                reason: format!("the file reference {element} has no resolved MIME type and URI"),
            });
        };
        return Ok(Element::File(file));
    }
    Ok(Element::Text(element))
}
pub fn build_part(
    element: &str,
    resolved_files: &ResolvedFiles,
) -> Result<Part, GeminiEmbeddingError> {
    Ok(match classify(element, resolved_files)? {
        Element::Text(text) => Part::Text(text.to_owned()),
        Element::Inline(data_url) => Part::InlineData(Blob {
            data: data_url.payload.to_owned(),
            mime_type: data_url.mime_type.to_owned(),
        }),
        Element::Gcs(mime_type, uri) => Part::FileData(FileData {
            mime_type: Some(mime_type.to_owned()),
            file_uri: uri.to_owned(),
        }),
        Element::File(file) => Part::FileData(FileData {
            mime_type: Some(file.mime_type.clone()),
            file_uri: file.uri.clone(),
        }),
    })
}
#[must_use]
pub fn flatten(input: &EmbeddingInput) -> Vec<&str> {
    let mut elements = Vec::new();
    match input {
        EmbeddingInput::Text(text) => elements.push(text.as_str()),
        EmbeddingInput::Entries(entries) => {
            for entry in entries {
                match entry {
                    InputEntry::Text(text) => elements.push(text.as_str()),
                    InputEntry::Group(group) => elements.extend(group.iter().map(String::as_str)),
                }
            }
        }
    }
    elements
}
#[must_use]
pub fn is_image_only(input: &EmbeddingInput, resolved_files: &ResolvedFiles) -> bool {
    let elements = flatten(input);
    !elements.is_empty()
        && elements
            .iter()
            .all(|element| match classify(element, resolved_files) {
                Ok(Element::Inline(data_url)) => IMAGE_MIME_TYPES.contains(&data_url.mime_type),
                Ok(Element::Gcs(mime_type, _)) => IMAGE_MIME_TYPES.contains(&mime_type),
                Ok(Element::File(file)) => IMAGE_MIME_TYPES.contains(&file.mime_type.as_str()),
                Ok(Element::Text(_)) | Err(_) => false,
            })
}
