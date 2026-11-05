use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use http::Method;
use secrecy::SecretString;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::files::error::{FilesError, StatusError};
use crate::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileObject, FileQuery, decode_file_object,
};
use crate::files::request::file_path;
use crate::files::stream::{FileContentStreamingResult, read_error_body};
use crate::files::transport::FileTransport;

pub mod error;
pub mod records;
pub mod request;
pub mod stream;
pub mod transport;

type JsonReply = Result<Value, FilesError>;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1/";

// the OpenAI SDK's default request timeout
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

// the OpenAI SDK's default retry count
pub const DEFAULT_MAX_RETRIES: u32 = 2;

// The arguments LiteLLM's get_openai_client forwards to the SDK constructor; None is left out of the call
#[derive(Debug, Clone, Default)]
pub struct ClientArgs {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Duration>,
    pub max_retries: Option<u32>,
    pub organization: Option<String>,
}

// betterlite never retries on its own (B173), so max_retries is only carried for callers that report it
#[derive(Debug, Clone)]
pub struct FilesClient {
    pub api_key: SecretString,
    pub base_url: Url,
    pub organization: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
}

#[derive(Debug, Clone)]
pub enum ClientKind {
    Sync(FilesClient),
    Async(FilesClient),
}

// A supplied client wins; otherwise one is built only when a key is present, as the SDK would need one
pub fn get_openai_client(
    args: ClientArgs,
    supplied: Option<ClientKind>,
    is_async: bool,
) -> Result<Option<ClientKind>, FilesError> {
    if supplied.is_some() {
        return Ok(supplied);
    }
    let Some(api_key) = args.api_key else {
        return Ok(None);
    };
    let mut base_url = Url::parse(args.api_base.as_deref().unwrap_or(DEFAULT_BASE_URL))
        .map_err(|source| FilesError::BaseUrl { source })?;
    if !base_url.path().ends_with('/') {
        let path = format!("{}/", base_url.path());
        base_url.set_path(&path);
    }
    let client = FilesClient {
        api_key,
        base_url,
        organization: args.organization,
        timeout: args.timeout.unwrap_or(DEFAULT_TIMEOUT),
        max_retries: args.max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
    };
    Ok(Some(if is_async {
        ClientKind::Async(client)
    } else {
        ClientKind::Sync(client)
    }))
}

// The checks every OpenAIFilesAPI method makes on what get_openai_client returned
pub fn select_client(
    client: Option<ClientKind>,
    is_async: bool,
) -> Result<FilesClient, FilesError> {
    match (client, is_async) {
        (None, _) => Err(FilesError::NotInitialized),
        (Some(ClientKind::Sync(_)), true) => Err(FilesError::NotAsync),
        (Some(ClientKind::Sync(chosen) | ClientKind::Async(chosen)), _) => Ok(chosen),
    }
}

pub struct FilesApi<T> {
    transport: T,
    boundaries: fn() -> Uuid,
}

impl<T: FileTransport> FilesApi<T> {
    pub const fn new(transport: T, boundaries: fn() -> Uuid) -> Self {
        Self {
            transport,
            boundaries,
        }
    }

    async fn buffered(&self, request: OutboundRequest) -> Result<UpstreamResponse, FilesError> {
        let reply = self
            .transport
            .send_buffered(request)
            .await
            .map_err(|source| FilesError::Transport { source })?;
        if !(200..300).contains(&reply.status) {
            let failure = StatusError::from_response(reply.status, &reply.body);
            return Err(FilesError::Status(Box::new(failure)));
        }
        Ok(reply)
    }

    pub async fn create_file(
        &self,
        client: &FilesClient,
        data: &FileCreate,
    ) -> Result<FileObject, FilesError> {
        let boundary = (self.boundaries)().simple().to_string();
        let reply = self
            .buffered(client.create_request(data, boundary)?)
            .await?;
        decode_file_object(&reply.body)
    }

    pub async fn file_content(
        &self,
        client: &FilesClient,
        request: &FileContentRequest,
    ) -> Result<BinaryContent, FilesError> {
        Ok(BinaryContent::from_response(
            self.buffered(client.content_request(request)?).await?,
        ))
    }

    // The response context is entered before this returns; the stream owns leaving it
    pub async fn file_content_streaming(
        &self,
        client: &FilesClient,
        request: &FileContentRequest,
        chunk_size: Option<NonZeroUsize>,
    ) -> Result<FileContentStreamingResult, FilesError> {
        let reply = self
            .transport
            .send_stream(client.content_request(request)?)
            .await
            .map_err(|source| FilesError::Transport { source })?;
        if !(200..300).contains(&reply.status) {
            let body = read_error_body(reply.body).await;
            let failure = StatusError::from_response(reply.status, &body);
            return Err(FilesError::Status(Box::new(failure)));
        }
        Ok(FileContentStreamingResult::new(
            reply.body,
            &reply.headers,
            chunk_size,
        ))
    }

    async fn json_call(
        &self,
        client: &FilesClient,
        method: Method,
        path: String,
        query: Vec<(String, String)>,
    ) -> JsonReply {
        let reply = self
            .buffered(client.bare_request(method, path, query)?)
            .await?;
        serde_json::from_slice(&reply.body).map_err(|source| FilesError::Decode { source })
    }

    pub async fn retrieve_file(&self, client: &FilesClient, file_id: &str) -> JsonReply {
        self.json_call(client, Method::GET, file_path(file_id, "")?, Vec::new())
            .await
    }

    pub async fn delete_file(&self, client: &FilesClient, file_id: &str) -> JsonReply {
        self.json_call(client, Method::DELETE, file_path(file_id, "")?, Vec::new())
            .await
    }

    // Only purpose is forwarded; after, limit and order are not arguments of this wrapper
    pub async fn list_files(&self, client: &FilesClient, query: &FileQuery) -> JsonReply {
        self.json_call(client, Method::GET, "/files".to_owned(), query.pairs())
            .await
    }
}

mod tests;
