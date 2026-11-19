use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_openai::files::records::{FileKind, FileObject, FileStatus, FilesPurpose};

pub const FILE_ID_PREFIX: &str = "file-";
pub const FILE_ID_DIGITS: usize = 24;

// LiteLLM keeps these on OpenAIFileObject._hidden_params, which its public fields never carry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub file: FileObject,
    pub storage_backend: String,
    pub storage_url: String,
}

#[derive(Clone, Copy)]
pub struct FileMetadata<'data> {
    pub content: &'data [u8],
    pub filename: &'data str,
    pub purpose: FilesPurpose,
    pub target_storage: &'data str,
    pub storage_url: &'data str,
}

#[must_use]
pub fn project_stored_file(
    metadata: FileMetadata<'_>,
    clock: &dyn Clock,
    ids: &dyn IdSource,
) -> StoredFile {
    let file = FileObject {
        id: file_id(ids),
        bytes: i64::try_from(metadata.content.len()).unwrap_or(i64::MAX),
        created_at: i64::try_from(clock.now_unix().as_secs()).unwrap_or(i64::MAX),
        filename: metadata.filename.to_owned(),
        object: FileKind::File,
        purpose: metadata.purpose,
        status: Some(FileStatus::Uploaded),
        expires_at: None,
        status_details: None,
    };
    StoredFile {
        file,
        storage_backend: metadata.target_storage.to_owned(),
        storage_url: metadata.storage_url.to_owned(),
    }
}

fn file_id(ids: &dyn IdSource) -> String {
    let digits: String = ids
        .next_id()
        .chars()
        .filter(|candidate| *candidate != '-')
        .take(FILE_ID_DIGITS)
        .collect();
    format!("{FILE_ID_PREFIX}{digits}")
}
