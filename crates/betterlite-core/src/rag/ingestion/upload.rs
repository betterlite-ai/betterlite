use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use thiserror::Error;

use crate::rag::upload_security::MAX_UPLOAD_SIZE_BYTES;

pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

// LiteLLM bounds no download; the RAG upload ceiling is the largest file the pipeline accepts
pub const MAX_FETCHED_FILE_BYTES: usize = MAX_UPLOAD_SIZE_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileData {
    pub filename: String,
    pub content: Vec<u8>,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub filename: Option<String>,
    pub file_content: Option<Vec<u8>>,
    pub content_type: Option<String>,
    pub existing_file_id: Option<String>,
}

#[derive(Debug)]
pub struct FetchedResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait UrlFetch: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<FetchedResponse, UrlFetchError>;
}

#[derive(Debug, Error)]
pub enum UrlFetchError {
    #[error("the file URL request failed")]
    Request(#[source] Box<UserUrlError>),
    #[error("the fetched file could not be read")]
    Read(#[source] Box<TransportError>),
    #[error("the file URL fetch failed: {message}")]
    Failed { message: String },
}

pub struct HttpUrlFetch<R> {
    pub client: ProxiedClient,
    pub urls: UserUrlFetcher<R>,
}

#[async_trait]
impl<R> UrlFetch for HttpUrlFetch<R>
where
    R: Resolve + 'static,
{
    async fn fetch(&self, url: &str) -> Result<FetchedResponse, UrlFetchError> {
        let fetched: StreamResponse = self
            .urls
            .fetch(&self.client, url, &HeaderMap::new())
            .await
            .map_err(|source| UrlFetchError::Request(Box::new(source)))?;
        let response = fetched
            .buffered(MAX_FETCHED_FILE_BYTES)
            .await
            .map_err(|source| UrlFetchError::Read(Box::new(source)))?;
        Ok(FetchedResponse {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }
}

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("a file source is required: file_data, file_url, or file_id")]
    MissingSource,
    #[error("the file URL could not be fetched")]
    Fetch(#[from] UrlFetchError),
    #[error("the file URL returned HTTP status {status}")]
    HttpStatus { status: u16 },
}

#[must_use]
pub fn filename_from_url(url: &str) -> String {
    let last = url.rsplit('/').next().unwrap_or_default();
    if last.is_empty() {
        "document".to_owned()
    } else {
        last.to_owned()
    }
}

#[must_use]
pub fn content_type_from_headers(headers: &HeaderMap) -> String {
    let Some(value) = headers.get(CONTENT_TYPE) else {
        return DEFAULT_CONTENT_TYPE.to_owned();
    };
    value
        .as_bytes()
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}
