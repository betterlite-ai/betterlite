pub mod client;
pub mod error;
mod patched;

use betterlite_openai::files::FilesApi;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileKind, FileObject, FilePage,
    FileQuery,
};
use betterlite_openai::files::transport::FileTransport;
use indexmap::IndexMap;
use uuid::Uuid;

use client::{ClientMaterializer, Connection, SelectedClient};
use error::AzureFilesError;
use patched::Patched;

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

pub async fn create_file<T, M>(
    connection: Connection<'_, '_, M>,
    transport: &T,
    data: &FileCreate,
) -> Result<FileObject, AzureFilesError>
where
    T: FileTransport,
    M: ClientMaterializer,
{
    let is_async = connection.is_async;
    let client = connection.open()?;
    // LiteLLM's two debug records hold the whole data and response objects; only their metadata is kept
    if is_async {
        tracing::debug!(
            purpose = data.purpose.wire_name(),
            filename = data.file.filename.as_deref().unwrap_or_default(),
            bytes = data.file.data.len(),
            expires_after = ?data.expires_after.map(|expires| expires.seconds),
            "create_file_data"
        );
    }
    let object = api(&client, transport, Some(&data.extra_headers))
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
    Ok(object)
}

pub async fn file_content<T, M>(
    connection: Connection<'_, '_, M>,
    transport: &T,
    request: &FileContentRequest,
) -> Result<BinaryContent, AzureFilesError>
where
    T: FileTransport,
    M: ClientMaterializer,
{
    let client = connection.open()?;
    api(&client, transport, Some(&request.extra_headers))
        .file_content(&client.files, request)
        .await
        .map_err(AzureFilesError::from)
}

pub async fn retrieve_file<T, M>(
    connection: Connection<'_, '_, M>,
    transport: &T,
    file_id: &str,
) -> Result<FileObject, AzureFilesError>
where
    T: FileTransport,
    M: ClientMaterializer,
{
    let client = connection.open()?;
    api(&client, transport, None)
        .retrieve_file(&client.files, file_id)
        .await
        .map_err(AzureFilesError::from)
}

pub async fn list_files<T, M>(
    connection: Connection<'_, '_, M>,
    transport: &T,
    purpose: Option<&str>,
) -> Result<FilePage, AzureFilesError>
where
    T: FileTransport,
    M: ClientMaterializer,
{
    let client = connection.open()?;
    let query = FileQuery {
        purpose: purpose.map(str::to_owned),
    };
    api(&client, transport, None)
        .list_files(&client.files, &query)
        .await
        .map_err(AzureFilesError::from)
}

pub async fn delete_file<T, M>(
    connection: Connection<'_, '_, M>,
    transport: &T,
    file_id: &str,
) -> Result<FileDeleted, AzureFilesError>
where
    T: FileTransport,
    M: ClientMaterializer,
{
    let client = connection.open()?;
    let reply = api(&client, transport, None)
        .delete_file(&client.files, file_id)
        .await;
    match reply {
        Ok(deleted) => Ok(deleted),
        // Azure answers delete with an empty body; LiteLLM replaces anything that is not a FileDeleted
        Err(FilesError::Decode { .. }) => Ok(FileDeleted {
            id: file_id.to_owned(),
            object: FileKind::File,
            deleted: true,
        }),
        Err(error) => Err(error.into()),
    }
}

mod tests;
