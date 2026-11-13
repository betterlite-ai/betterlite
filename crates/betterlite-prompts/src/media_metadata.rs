pub mod error;
mod mime;
pub mod url_mime;

use std::collections::BTreeMap;

use error::MediaMetadataError;

const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

// A filename is metadata, never permission to read a local path (B159-B161 own path/handle bodies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileBody {
    Bytes(Vec<u8>),
    BareString(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileData {
    Direct(FileBody),
    Tuple2 {
        filename: Option<String>,
        content: FileBody,
    },
    Tuple3 {
        filename: Option<String>,
        content: FileBody,
        content_type: Option<String>,
    },
    Tuple4 {
        filename: Option<String>,
        content: FileBody,
        content_type: Option<String>,
        headers: BTreeMap<String, String>,
    },
    InMemoryFile {
        name: String,
        content_type: String,
        content: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFileData {
    pub filename: Option<String>,
    pub content: Vec<u8>,
    pub content_type: String,
    pub headers: BTreeMap<String, String>,
}

struct Parts {
    filename: Option<String>,
    content: FileBody,
    content_type: Option<String>,
    headers: BTreeMap<String, String>,
}

impl Parts {
    const fn new(
        filename: Option<String>,
        content: FileBody,
        content_type: Option<String>,
        headers: BTreeMap<String, String>,
    ) -> Self {
        Self {
            filename,
            content,
            content_type,
            headers,
        }
    }
}

fn parts_of(file_data: &FileData) -> Parts {
    match file_data {
        FileData::Direct(content) => Parts::new(None, content.clone(), None, BTreeMap::new()),
        FileData::Tuple2 { filename, content } => {
            Parts::new(filename.clone(), content.clone(), None, BTreeMap::new())
        }
        FileData::Tuple3 {
            filename,
            content,
            content_type,
        } => Parts::new(
            filename.clone(),
            content.clone(),
            content_type.clone(),
            BTreeMap::new(),
        ),
        FileData::Tuple4 {
            filename,
            content,
            content_type,
            headers,
        } => Parts::new(
            filename.clone(),
            content.clone(),
            content_type.clone(),
            headers.clone(),
        ),
        FileData::InMemoryFile {
            name,
            content_type,
            content,
        } => Parts::new(
            Some(name.clone()),
            FileBody::Bytes(content.clone()),
            Some(content_type.clone()),
            BTreeMap::new(),
        ),
    }
}

fn resolved_content_type(declared: Option<&str>, filename: Option<&str>) -> String {
    if let Some(value) = declared
        && !value.is_empty()
    {
        return value.to_owned();
    }
    filename
        .filter(|name| !name.is_empty())
        .and_then(mime::guess_mime_type)
        .unwrap_or(DEFAULT_CONTENT_TYPE)
        .to_owned()
}

#[must_use]
pub fn extract_file_metadata(file_data: &FileData) -> (Option<String>, String) {
    let parts = parts_of(file_data);
    let content_type =
        resolved_content_type(parts.content_type.as_deref(), parts.filename.as_deref());
    (parts.filename, content_type)
}

// Rejects a bare string body: a caller-controlled string must not open an arbitrary path.
pub fn extract_file_data(file_data: &FileData) -> Result<ExtractedFileData, MediaMetadataError> {
    let parts = parts_of(file_data);
    let content = match parts.content {
        FileBody::Bytes(bytes) => bytes,
        FileBody::BareString(_) => return Err(MediaMetadataError::BareStringContent),
    };
    let content_type =
        resolved_content_type(parts.content_type.as_deref(), parts.filename.as_deref());
    Ok(ExtractedFileData {
        filename: parts.filename,
        content,
        content_type,
        headers: parts.headers,
    })
}

mod tests;
