use betterlite_core::api::file_contracts::is_batch_jsonl_file;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{
    FileBody, FileData, extract_file_data, extract_file_metadata,
};

use crate::vertex_batch_upload::sources::{FileContent, FileTypes};

use super::FileRequest;
use super::error::VertexFilesError;

const STREAM_CONTENT_TYPE: &str = "application/json";

// B1015 decides from the purpose and the MIME B235 resolved; the body is passed so the predicate sees the file it guards
pub trait BatchJsonlPredicate: Send + Sync {
    fn matches(&self, purpose: &str, content_type: &str, file: &FileTypes) -> bool;
}

pub struct BatchJsonlFiles;

impl BatchJsonlPredicate for BatchJsonlFiles {
    fn matches(&self, purpose: &str, content_type: &str, file: &FileTypes) -> bool {
        is_batch_jsonl_file(Some(purpose), Some(content_type), Some(file))
    }
}

// LiteLLM's bytes | str | dict return, split into the streaming media plan and the plain byte body
pub enum SelectedUpload {
    Passthrough(FileTypes),
    Converted(FileTypes),
    Bytes(Vec<u8>),
}

impl SelectedUpload {
    #[must_use]
    pub const fn content_type(&self) -> Option<&'static str> {
        match self {
            Self::Passthrough(_) | Self::Converted(_) => Some(STREAM_CONTENT_TYPE),
            Self::Bytes(_) => None,
        }
    }
}

fn file_content(body: &FileBody) -> FileContent {
    match body {
        FileBody::Bytes(bytes) => FileContent::Bytes(bytes.clone()),
        FileBody::BareString(text) => FileContent::Text(text.clone()),
    }
}

fn named(filename: Option<&str>, content: &FileBody) -> FileTypes {
    let Some(name) = filename.filter(|name| !name.is_empty()) else {
        return FileTypes::Content(file_content(content));
    };
    FileTypes::Tuple {
        filename: name.to_owned(),
        content: file_content(content),
    }
}

// the upload stream reads only the content, so the MIME and headers of a tuple stay behind
fn file_types(file: &FileData) -> FileTypes {
    match file {
        FileData::Direct(content) => FileTypes::Content(file_content(content)),
        FileData::Tuple2 { filename, content }
        | FileData::Tuple3 {
            filename, content, ..
        }
        | FileData::Tuple4 {
            filename, content, ..
        } => named(filename.as_deref(), content),
        FileData::InMemoryFile { name, content, .. } => FileTypes::Tuple {
            filename: name.clone(),
            content: FileContent::Bytes(content.clone()),
        },
    }
}

pub fn select_upload(
    request: &FileRequest<'_>,
    predicate: &dyn BatchJsonlPredicate,
) -> Result<SelectedUpload, VertexFilesError> {
    let file = file_types(request.file);
    if request.purpose == CreatePurpose::Batch && request.passthrough {
        return Ok(SelectedUpload::Passthrough(file));
    }
    let (_filename, content_type) = extract_file_metadata(request.file);
    if predicate.matches(request.purpose.wire_name(), content_type.as_str(), &file) {
        return Ok(SelectedUpload::Converted(file));
    }
    Ok(SelectedUpload::Bytes(
        extract_file_data(request.file)?.content,
    ))
}

mod tests;
