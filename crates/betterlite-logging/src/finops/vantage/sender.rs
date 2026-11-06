use std::error::Error;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{AUTHORIZATION, HeaderValue, InvalidHeaderValue};
use http::{HeaderMap, Method};
use secrecy::{ExposeSecret as _, SecretString};
use url::{ParseError, Url};
use uuid::Uuid;

pub const CSV_FIELD_NAME: &str = "csv";
pub const CSV_MEDIA_TYPE: &str = "text/csv";
// litellm.request_timeout when it is set, otherwise LiteLLM's fallback request timeout
pub const DEFAULT_UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct MultipartPost {
    pub url: SecretString,
    pub authorization: SecretString,
    pub file: MultipartPart,
}

pub trait CsvSender: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post_multipart(
        &self,
        post: MultipartPost,
    ) -> impl Future<Output = Result<u16, Self::Error>> + Send;
}

pub struct HttpCsvSender {
    pub client: ProxiedClient,
    pub deadline: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("the Vantage upload URL is not valid")]
    Url(#[source] ParseError),
    #[error("the Authorization header holds a character a header cannot carry")]
    Authorization(#[source] InvalidHeaderValue),
    #[error("the upload could not be sent")]
    Transport(#[source] TransportError),
}

impl CsvSender for HttpCsvSender {
    type Error = UploadError;

    async fn post_multipart(&self, post: MultipartPost) -> Result<u16, UploadError> {
        let url = Url::parse(post.url.expose_secret()).map_err(UploadError::Url)?;
        let authorization = HeaderValue::try_from(post.authorization.expose_secret())
            .map_err(UploadError::Authorization)?;
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let request = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Multipart {
                boundary: Uuid::new_v4().to_string(),
                parts: vec![post.file],
            },
            deadline: self.deadline,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(UploadError::Transport)?;
        Ok(response.status)
    }
}
