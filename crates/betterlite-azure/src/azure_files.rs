pub mod client;
pub mod error;
mod patched;

use std::sync::{Mutex, PoisonError};

use betterlite_openai::files::FilesApi;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileKind, FileObject, FilePage,
    FileQuery,
};
use betterlite_openai::files::transport::FileTransport;
use indexmap::IndexMap;
use serde_json::error::Category;
use uuid::Uuid;

use client::{Opened, SelectedClient};
use error::AzureFilesError;
use patched::{Patched, Recording};

fn api<'client, T>(
    client: &'client SelectedClient,
    transport: &'client T,
    extra_headers: Option<&'client IndexMap<String, String>>,
) -> FilesApi<Patched<'client, T>> {
    FilesApi {
        transport: Patched {
            client,
            inner: transport,
            extra_headers,
        },
        boundaries: Uuid::new_v4,
    }
}

// The five file calls LiteLLM's handler exposes; each one sends through the same wire client
pub enum Operation<'data> {
    Create(&'data FileCreate),
    Content(&'data FileContentRequest),
    Retrieve(&'data str),
    List(Option<&'data str>),
    Delete(&'data str),
}

// The SDK reply of each call, which the handler hands back as it received it
pub enum Reply {
    Object(FileObject),
    Content(BinaryContent),
    Page(FilePage),
    Deleted(FileDeleted),
}

pub async fn call<T>(
    opened: &Opened,
    transport: &T,
    operation: Operation<'_>,
) -> Result<Reply, AzureFilesError>
where
    T: FileTransport,
{
    match operation {
        Operation::Create(data) => create(&opened.client, opened.is_async, transport, data).await,
        Operation::Content(request) => Ok(Reply::Content(
            api(&opened.client, transport, Some(&request.extra_headers))
                .file_content(&opened.client.files, request)
                .await?,
        )),
        Operation::Retrieve(file_id) => Ok(Reply::Object(
            api(&opened.client, transport, None)
                .retrieve_file(&opened.client.files, file_id)
                .await?,
        )),
        Operation::List(purpose) => {
            let query = FileQuery {
                purpose: purpose.map(str::to_owned),
            };
            Ok(Reply::Page(
                api(&opened.client, transport, None)
                    .list_files(&opened.client.files, &query)
                    .await?,
            ))
        }
        Operation::Delete(file_id) => delete(&opened.client, transport, file_id).await,
    }
}

// LiteLLM's two debug records hold the whole data and response objects; only their metadata is kept
async fn create<T>(
    client: &SelectedClient,
    is_async: bool,
    transport: &T,
    data: &FileCreate,
) -> Result<Reply, AzureFilesError>
where
    T: FileTransport,
{
    if is_async {
        tracing::debug!(
            purpose = data.purpose.wire_name(),
            filename = data.file.filename.as_deref().unwrap_or_default(),
            bytes = data.file.data.len(),
            expires_after = ?data.expires_after.map(|expires| expires.seconds),
            "create_file_data"
        );
    }
    let object = api(client, transport, Some(&data.extra_headers))
        .create_file(&client.files, data)
        .await?;
    if is_async {
        tracing::debug!(
            id = %object.id,
            bytes = object.bytes,
            purpose = ?object.purpose,
            status = ?object.status,
            "create_file_response"
        );
    }
    Ok(Reply::Object(object))
}

// Azure answers delete with an empty body; LiteLLM replaces anything that is not a FileDeleted
async fn delete<T>(
    client: &SelectedClient,
    transport: &T,
    file_id: &str,
) -> Result<Reply, AzureFilesError>
where
    T: FileTransport,
{
    let observed = Mutex::new(None);
    let calls = FilesApi {
        transport: Recording {
            inner: Patched {
                client,
                inner: transport,
                extra_headers: None,
            },
            content_type: &observed,
        },
        boundaries: Uuid::new_v4,
    };
    let reply = calls.delete_file(&client.files, file_id).await;
    let content_type = observed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    Ok(Reply::Deleted(deleted_record(
        file_id,
        reply,
        content_type.as_deref(),
    )?))
}

// The SDK parses a reply as JSON only when its content type ends in json, and raises when that parse fails
fn deleted_record(
    file_id: &str,
    reply: Result<FileDeleted, FilesError>,
    content_type: Option<&str>,
) -> Result<FileDeleted, AzureFilesError> {
    match reply {
        Ok(record) => Ok(record),
        Err(FilesError::Decode { source })
            if matches!(source.classify(), Category::Syntax | Category::Eof)
                && content_type.is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .is_some_and(|kind| kind.ends_with("json"))
                }) =>
        {
            Err(AzureFilesError::from(FilesError::Decode { source }))
        }
        Err(FilesError::Decode { .. }) => Ok(FileDeleted {
            id: file_id.to_owned(),
            object: FileKind::File,
            deleted: true,
        }),
        Err(error) => Err(AzureFilesError::from(error)),
    }
}

mod tests;
