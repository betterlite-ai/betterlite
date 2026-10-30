use std::io::{self, ErrorKind};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use opentelemetry_http::{HttpClient, HttpError, Request, Response};
use reqwest::blocking::Client;
use rustls::ClientConfig;
use thiserror::Error;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum OtlpClientError {
    #[error("could not start the thread that builds the blocking HTTP client: {source}")]
    Thread {
        #[source]
        source: io::Error,
    },
    #[error("the thread that builds the blocking HTTP client panicked")]
    ThreadPanicked,
    #[error("could not build the blocking HTTP client: {source}")]
    Build {
        #[source]
        source: reqwest::Error,
    },
}

#[derive(Debug)]
pub struct OtlpClient {
    client: Client,
}

impl OtlpClient {
    pub fn build(tls: ClientConfig) -> Result<Self, OtlpClientError> {
        let handle = thread::Builder::new()
            .name("otlp-metrics-client".to_owned())
            .spawn(move || {
                Client::builder()
                    .tls_backend_preconfigured(tls)
                    .timeout(REQUEST_TIMEOUT)
                    .build()
            })
            .map_err(|source| OtlpClientError::Thread { source })?;
        let client = handle
            .join()
            .map_err(|_panic| OtlpClientError::ThreadPanicked)?
            .map_err(|source| OtlpClientError::Build { source })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl HttpClient for OtlpClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let (parts, request_body) = request.into_parts();
        let builder = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(request_body);
        let response = builder.send()?;
        let status = response.status();
        let headers = response.headers().clone();
        let response_body = response.bytes()?;
        if response_body.len() > MAX_RESPONSE_BYTES {
            return Err(Box::new(io::Error::new(
                ErrorKind::InvalidData,
                "OTLP response body exceeded 4 MiB",
            )));
        }
        let mut converted = Response::builder().status(status).body(response_body)?;
        *converted.headers_mut() = headers;
        Ok(converted)
    }
}
