use std::future::{Future, ready};
use std::mem::take;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::tls::apply_policy;
use http::{Method, StatusCode};
use reqwest::Response;
use tokio::time::timeout;

use crate::client::cleanup::http_client::{HttpResponse, ResponseHead, Transport};
use crate::client::cleanup::recording::ResponseBody;
use crate::client::sessions::factory::{HttpClientConfig, RequestCredential, TlsVerification};

use super::capture::{BodyChunks, CaptureRecord, ErrorCapture, Exchange};
use super::error::McpHttpError;
use super::request::{McpHttpRequest, PreparedRequest, prepare};

pub struct HttpBase {
    pub proxy: Arc<ProxyPolicy>,
    pub tls: TlsPolicy,
}

pub struct McpHttpTransport {
    config: HttpClientConfig,
    client: ProxiedClient,
    capture: Arc<dyn ErrorCapture>,
}

impl McpHttpTransport {
    pub fn new(
        config: &HttpClientConfig,
        base: &HttpBase,
        capture: Arc<dyn ErrorCapture>,
    ) -> Result<Self, McpHttpError> {
        let mut tls = base.tls.clone();
        match config.tls.verification {
            TlsVerification::Enabled => tls.verification = Verification::Enabled,
            TlsVerification::Disabled => {
                tls.verification = Verification::Disabled;
                tls.ca_file = None;
            }
            TlsVerification::CaFile => {
                let path = config.tls.ca_file.clone().unwrap_or_default();
                tls.verification = Verification::CaFile(path.clone());
                tls.ca_file = Some(path);
            }
        }
        // reqwest offers a connect timeout and a per-read timeout; the body applies the read one per request
        let limits = TransportLimits {
            connect_timeout: config.timeout,
            read_timeout: TransportLimits::DEFAULT.read_timeout,
            body_bytes: TransportLimits::DEFAULT.body_bytes,
            header_bytes: TransportLimits::DEFAULT.header_bytes,
        };
        let client =
            ProxiedClient::build(apply_policy(base_builder(), &tls)?, &base.proxy, limits)?;
        Ok(Self {
            config: config.clone(),
            client,
            capture,
        })
    }

    async fn execute(&self, prepared: &PreparedRequest) -> Result<Response, McpHttpError> {
        let mut builder = self
            .client
            .client
            .request(prepared.method.clone(), prepared.url.clone());
        builder = builder.headers(prepared.headers.clone());
        if !prepared.body.is_empty() {
            builder = builder.body(prepared.body.clone());
        }
        let request = builder.build().map_err(McpHttpError::Request)?;
        match self.client.execute(request).await {
            Ok(response) => Ok(response),
            Err(HttpError::Request(source)) => Err(send_failure(source)),
            Err(other) => Err(McpHttpError::Client(other)),
        }
    }

    async fn attempt(
        &self,
        prepared: &PreparedRequest,
        read_timeout: Duration,
    ) -> Result<HttpResponse<HttpBody>, McpHttpError> {
        let response = timeout(read_timeout, self.execute(prepared))
            .await
            .map_err(McpHttpError::HeadTimeout)??;
        let status = response.status().as_u16();
        if status >= 400 && self.config.capture_error_responses {
            return self
                .capture_response(response, prepared, read_timeout)
                .await;
        }
        let headers = response.headers().clone();
        Ok(HttpResponse {
            status,
            headers,
            capture: None,
            body: HttpBody {
                state: BodyState::Live(Box::new(response)),
                read_timeout,
            },
        })
    }

    async fn capture_response(
        &self,
        response: Response,
        prepared: &PreparedRequest,
        read_timeout: Duration,
    ) -> Result<HttpResponse<HttpBody>, McpHttpError> {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let mut body = HttpBody {
            state: BodyState::Live(Box::new(response)),
            read_timeout,
        };
        let exchange = Exchange {
            method: prepared.method.clone(),
            url: prepared.url.clone(),
            request_headers: prepared.headers.clone(),
            request_body: prepared.body.clone(),
            status,
            response_headers: headers.clone(),
        };
        let limit = TransportLimits::DEFAULT.body_bytes;
        let captured = self
            .capture
            .capture(
                exchange.clone(),
                BodyChunks {
                    body: &mut body,
                    limit,
                    remaining: limit,
                },
            )
            .await;
        Ok(HttpResponse {
            status,
            headers,
            capture: Some(CaptureRecord {
                exchange,
                preview: captured.preview,
            }),
            body: HttpBody {
                state: BodyState::Buffered(captured.retained),
                read_timeout,
            },
        })
    }
}

impl Transport for McpHttpTransport {
    type Request = McpHttpRequest;
    type Body = HttpBody;
    type Error = McpHttpError;

    fn method(request: &McpHttpRequest) -> &Method {
        &request.method
    }

    async fn send(
        &self,
        request: McpHttpRequest,
        _stream: bool,
    ) -> Result<HttpResponse<HttpBody>, McpHttpError> {
        let read_timeout = request.read_timeout.unwrap_or(self.config.timeout);
        let prepared = prepare(&self.config, &request).await?;
        let first = self.attempt(&prepared, read_timeout).await?;
        let Some(RequestCredential::Resolved(auth)) = self.config.credential.as_ref() else {
            return Ok(first);
        };
        if first.status != StatusCode::UNAUTHORIZED.as_u16() || !auth.after_unauthorized().await {
            return Ok(first);
        }
        let retried = prepare(&self.config, &request).await?;
        self.attempt(&retried, read_timeout).await
    }

    fn status_error(&self, head: ResponseHead) -> McpHttpError {
        McpHttpError::Status(Box::new(head))
    }
}

pub struct HttpBody {
    state: BodyState,
    read_timeout: Duration,
}

enum BodyState {
    Live(Box<Response>),
    Buffered(Vec<u8>),
}

impl ResponseBody for HttpBody {
    type Error = McpHttpError;

    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, McpHttpError>> {
        match &mut self.state {
            BodyState::Buffered(content) => {
                let bytes = take(content);
                (!bytes.is_empty()).then_some(Ok(bytes))
            }
            BodyState::Live(response) => match timeout(self.read_timeout, response.chunk()).await {
                Ok(Ok(Some(chunk))) => Some(Ok(chunk.to_vec())),
                Ok(Ok(None)) => None,
                Ok(Err(source)) => Some(Err(McpHttpError::Body(source))),
                Err(elapsed) => Some(Err(McpHttpError::BodyTimeout(elapsed))),
            },
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), McpHttpError>> + Send {
        self.state = BodyState::Buffered(Vec::new());
        ready(Ok(()))
    }

    async fn read_all(&mut self) -> Result<Vec<u8>, McpHttpError> {
        let mut content = Vec::new();
        while let Some(chunk) = self.next_chunk().await {
            content.extend(chunk?);
        }
        Ok(content)
    }
}

fn send_failure(source: reqwest::Error) -> McpHttpError {
    if source.is_timeout() {
        return McpHttpError::Timeout(source);
    }
    if source.is_connect() {
        return McpHttpError::Connect(source);
    }
    McpHttpError::Request(source)
}
