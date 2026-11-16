use std::fmt;
use std::num::NonZeroUsize;

use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileObject, FilePage, FileQuery,
};
use betterlite_openai::files::stream::FileContentStreamingResult;
use betterlite_openai::files::transport::FileTransport;
use betterlite_openai::files::{FilesApi, FilesClient};

pub struct Dispatch<'a, T>
where
    T: FileTransport,
{
    pub api: &'a FilesApi<T>,
    pub client: &'a FilesClient,
    pub call: Call<'a>,
}

// B516's six lifecycle calls; the streaming call keeps its chunk size and leaves the stream to the caller
pub enum Call<'a> {
    Create(&'a FileCreate),
    Retrieve(&'a str),
    Delete(&'a str),
    List(&'a FileQuery),
    Content(&'a FileContentRequest),
    StreamingContent {
        request: &'a FileContentRequest,
        chunk_size: Option<NonZeroUsize>,
    },
}

pub enum Outcome {
    Object(Box<FileObject>),
    Deleted(Box<FileDeleted>),
    Page(Box<FilePage>),
    Content(Box<BinaryContent>),
    Streaming(Box<FileContentStreamingResult>),
}

// The streaming reply holds a response body and its exit flag, so only the variant is shown
impl fmt::Debug for Outcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Object(file) => formatter.debug_tuple("Object").field(file).finish(),
            Self::Deleted(deleted) => formatter.debug_tuple("Deleted").field(deleted).finish(),
            Self::Page(page) => formatter.debug_tuple("Page").field(page).finish(),
            Self::Content(content) => formatter.debug_tuple("Content").field(content).finish(),
            Self::Streaming(_) => formatter.write_str("Streaming(..)"),
        }
    }
}

impl<T: FileTransport> Dispatch<'_, T> {
    #[must_use]
    pub async fn run(self) -> Result<Outcome, FilesError> {
        let Self { api, client, call } = self;
        Ok(match call {
            Call::Create(data) => Outcome::Object(Box::new(api.create_file(client, data).await?)),
            Call::Retrieve(file_id) => {
                Outcome::Object(Box::new(api.retrieve_file(client, file_id).await?))
            }
            Call::Delete(file_id) => {
                Outcome::Deleted(Box::new(api.delete_file(client, file_id).await?))
            }
            Call::List(query) => Outcome::Page(Box::new(api.list_files(client, query).await?)),
            Call::Content(request) => {
                Outcome::Content(Box::new(api.file_content(client, request).await?))
            }
            Call::StreamingContent {
                request,
                chunk_size,
            } => Outcome::Streaming(Box::new(
                api.file_content_streaming(client, request, chunk_size)
                    .await?,
            )),
        })
    }
}
