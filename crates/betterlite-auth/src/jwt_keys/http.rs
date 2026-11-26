use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use thiserror::Error;

use super::MAX_DOCUMENT_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: Vec<u8>,
}

// httpx.TransportError alone is retried; a status, a parse failure and a cache error are answers, not outages
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("the identity provider could not be reached")]
    Transport(#[source] HttpError),
    #[error("the identity provider request failed")]
    Failed(#[source] HttpError),
}

#[async_trait]
pub trait JwksHttp: Send + Sync {
    async fn get(&self, url: &str) -> Result<HttpReply, FetchError>;
}

pub struct ProxiedJwksHttp {
    pub client: ProxiedClient,
    pub body_limit: usize,
}

impl ProxiedJwksHttp {
    #[must_use]
    pub fn new(client: ProxiedClient) -> Self {
        Self {
            client,
            body_limit: MAX_DOCUMENT_BYTES,
        }
    }
}

#[async_trait]
impl JwksHttp for ProxiedJwksHttp {
    async fn get(&self, url: &str) -> Result<HttpReply, FetchError> {
        // an unparsable configured URL is httpx.InvalidURL, which is a transport error and takes the retries
        let request = self
            .client
            .client
            .get(url)
            .build()
            .map_err(|source| FetchError::Transport(HttpError::Request(source)))?;
        match self.client.execute_buffered(request, self.body_limit).await {
            Ok(response) => Ok(HttpReply {
                status: response.status,
                body: response.body,
            }),
            Err(HttpError::Request(source)) => {
                Err(FetchError::Transport(HttpError::Request(source)))
            }
            Err(HttpError::Timeout) => Err(FetchError::Transport(HttpError::Timeout)),
            Err(
                other @ (HttpError::Required { .. }
                | HttpError::DotSegment { .. }
                | HttpError::UnsupportedProxyScheme { .. }
                | HttpError::InvalidProxyUrl { .. }
                | HttpError::Client(_)
                | HttpError::TooLarge { .. }),
            ) => Err(FetchError::Failed(other)),
        }
    }
}
