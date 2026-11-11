use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::gemini_embedding_records::data_url::{
    decode_embedding_data_url, infer_mime_from_gcs_url,
};
use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_media::part::{Blob, FileData};

pub const IMAGE_MIME_TYPES: [&str; 2] = ["image/jpeg", "image/png"];
const DATA_URL_MARKER: &str = ";base64,";
const FILE_REFERENCE_PREFIX: &str = "files/";
const GCS_PREFIX: &str = "gs://";

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

pub fn build_part(
    element: &str,
    resolved_files: &ResolvedFiles,
) -> Result<Part, GeminiEmbeddingError> {
    if element.starts_with("data:") && element.contains(DATA_URL_MARKER) {
        let data_url = decode_embedding_data_url(element)?;
        return Ok(Part::InlineData(Blob {
            data: data_url.payload.to_owned(),
            mime_type: data_url.mime_type.to_owned(),
        }));
    }
    if element.starts_with(GCS_PREFIX) {
        let mime_type = infer_mime_from_gcs_url(element)?;
        return Ok(Part::FileData(FileData {
            mime_type: Some(mime_type.to_owned()),
            file_uri: element.to_owned(),
        }));
    }
    if element.starts_with(FILE_REFERENCE_PREFIX) {
        let file =
            resolved_files
                .get(element)
                .ok_or_else(|| GeminiEmbeddingError::UnresolvedFile {
                    reference: element.to_owned(),
                })?;
        return Ok(Part::FileData(FileData {
            mime_type: Some(file.mime_type.clone()),
            file_uri: file.uri.clone(),
        }));
    }
    Ok(Part::Text(element.to_owned()))
}

#[must_use]
pub fn is_multimodal_element(element: &str) -> bool {
    (element.starts_with("data:") && element.contains(DATA_URL_MARKER))
        || element.starts_with(FILE_REFERENCE_PREFIX)
        || element.starts_with(GCS_PREFIX)
}

#[must_use]
pub fn is_multimodal(input: &EmbeddingInput) -> bool {
    match input {
        EmbeddingInput::Text(text) => is_multimodal_element(text),
        EmbeddingInput::Entries(entries) => entries.iter().any(|entry| match entry {
            InputEntry::Text(text) => is_multimodal_element(text),
            InputEntry::Group(group) => group.iter().any(|element| is_multimodal_element(element)),
        }),
    }
}

#[must_use]
pub fn has_nested(input: &EmbeddingInput) -> bool {
    match input {
        EmbeddingInput::Text(_) => false,
        EmbeddingInput::Entries(entries) => entries
            .iter()
            .any(|entry| matches!(entry, InputEntry::Group(_))),
    }
}

#[must_use]
pub fn flatten(input: &EmbeddingInput) -> Vec<&str> {
    match input {
        EmbeddingInput::Text(text) => vec![text.as_str()],
        EmbeddingInput::Entries(entries) => {
            let mut elements = Vec::with_capacity(entries.len());
            for entry in entries {
                match entry {
                    InputEntry::Text(text) => elements.push(text.as_str()),
                    InputEntry::Group(group) => elements.extend(group.iter().map(String::as_str)),
                }
            }
            elements
        }
    }
}

#[must_use]
pub fn is_image_element(element: &str, resolved_files: &ResolvedFiles) -> bool {
    if element.starts_with("data:") && element.contains(DATA_URL_MARKER) {
        return decode_embedding_data_url(element)
            .is_ok_and(|data_url| IMAGE_MIME_TYPES.contains(&data_url.mime_type));
    }
    if element.starts_with(GCS_PREFIX) {
        return infer_mime_from_gcs_url(element)
            .is_ok_and(|mime_type| IMAGE_MIME_TYPES.contains(&mime_type));
    }
    if element.starts_with(FILE_REFERENCE_PREFIX) {
        return resolved_files
            .get(element)
            .is_some_and(|file| IMAGE_MIME_TYPES.contains(&file.mime_type.as_str()));
    }
    false
}

#[must_use]
pub fn is_image_only(input: &EmbeddingInput, resolved_files: &ResolvedFiles) -> bool {
    let elements = flatten(input);
    !elements.is_empty()
        && elements
            .iter()
            .all(|element| is_image_element(element, resolved_files))
}
