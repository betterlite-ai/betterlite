use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use thiserror::Error;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct PostReply {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum Agent365PosterError {
    #[error("the Agent 365 request to {url} failed")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("the Agent 365 response from {url} could not be read")]
    Body {
        url: String,
        #[source]
        source: ReadError,
    },
    #[error("the Agent 365 poster has no reply for {url}")]
    NoReply { url: String },
}

// A non-2xx status is a reply and not an error: LiteLLM's raise_for_status is caught where the exchange inspects the status.
#[async_trait]
pub trait Agent365Poster: Send + Sync {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Duration,
    ) -> Result<PostReply, Agent365PosterError>;
}

pub struct HttpPoster {
    pub client: reqwest::Client,
}

#[async_trait]
impl Agent365Poster for HttpPoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Duration,
    ) -> Result<PostReply, Agent365PosterError> {
        let mut response = self
            .client
            .post(url)
            .headers(headers.clone())
            .body(body.to_vec())
            .timeout(timeout)
            .send()
            .await
            .map_err(|source| Agent365PosterError::Transport {
                url: url.to_owned(),
                source,
            })?;
        let status = response.status();
        let bytes = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|source| Agent365PosterError::Body {
                url: url.to_owned(),
                source,
            })?;
        Ok(PostReply {
            status,
            body: bytes,
        })
    }
}
