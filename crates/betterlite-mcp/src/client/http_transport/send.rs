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
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::{HeaderMap, Method, StatusCode};
use tokio::time::timeout;

use crate::client::cleanup::http_client::{HttpResponse, ResponseHead, Transport};
use crate::client::cleanup::recording::ResponseBody;
use crate::client::sessions::factory::{HttpClientConfig, RequestCredential, TlsVerification};

use super::capture::{BodyChunks, CaptureRecord, CapturedBody, ErrorCapture, Exchange};
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

struct LiveResponse {
    status: u16,
    headers: HeaderMap,
    stream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
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

    async fn execute(&self, prepared: &PreparedRequest) -> Result<LiveResponse, McpHttpError> {
        let mut builder = self
            .client
            .client
            .request(prepared.method.clone(), prepared.url.clone());
        builder = builder.headers(prepared.headers.clone());
        if !prepared.body.is_empty() {
            builder = builder.body(prepared.body.clone());
        }
        let request = builder
            .build()
            .map_err(|source| McpHttpError::Request(HttpError::Request(source)))?;
        match self.client.execute(request).await {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let headers = take(response.headers_mut());
                let stream = response
                    .bytes_stream()
                    .map(|chunk| {
                        chunk
                            .map(|bytes| bytes.to_vec())
                            .map_err(HttpError::Request)
                    })
                    .boxed();
                Ok(LiveResponse {
                    status,
                    headers,
                    stream,
                })
            }
            Err(error) => Err(send_failure(error)),
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
        if response.status >= 400 && self.config.capture_error_responses {
            return self
                .capture_response(response, prepared, read_timeout)
                .await;
        }
        Ok(HttpResponse {
            status: response.status,
            headers: response.headers,
            capture: None,
            body: HttpBody {
                state: BodyState::Live(response.stream),
                read_timeout,
                body_limit: TransportLimits::DEFAULT.body_bytes,
            },
        })
    }

    async fn capture_response(
        &self,
        response: LiveResponse,
        prepared: &PreparedRequest,
        read_timeout: Duration,
    ) -> Result<HttpResponse<HttpBody>, McpHttpError> {
        let LiveResponse {
            status,
            headers,
            stream,
        } = response;
        let mut body = HttpBody {
            state: BodyState::Live(stream),
            read_timeout,
            body_limit: TransportLimits::DEFAULT.body_bytes,
        };
        let exchange = Exchange {
            method: prepared.method.clone(),
            url: prepared.url.clone(),
            request_headers: prepared.headers.clone(),
            request_body: prepared.body.clone(),
            status,
            response_headers: headers.clone(),
        };
        let captured = self.capture_body(&mut body, &exchange).await;
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
                body_limit: TransportLimits::DEFAULT.body_bytes,
            },
        })
    }

    async fn capture_body(&self, body: &mut HttpBody, exchange: &Exchange) -> CapturedBody {
        let limit = TransportLimits::DEFAULT.body_bytes;
        self.capture
            .capture(
                exchange.clone(),
                BodyChunks {
                    body,
                    limit,
                    remaining: limit,
                },
            )
            .await
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
    pub(super) state: BodyState,
    pub(super) read_timeout: Duration,
    pub(super) body_limit: usize,
}

pub(super) enum BodyState {
    Live(BoxStream<'static, Result<Vec<u8>, HttpError>>),
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
            BodyState::Live(stream) => match timeout(self.read_timeout, stream.next()).await {
                Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
                Ok(Some(Err(error))) => Some(Err(McpHttpError::Body(error))),
                Ok(None) => None,
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
            let bytes = chunk?;
            let total =
                content
                    .len()
                    .checked_add(bytes.len())
                    .ok_or(McpHttpError::BodyTooLarge {
                        limit: self.body_limit,
                    })?;
            if total > self.body_limit {
                return Err(McpHttpError::BodyTooLarge {
                    limit: self.body_limit,
                });
            }
            content.extend(bytes);
        }
        Ok(content)
    }
}

fn send_failure(error: HttpError) -> McpHttpError {
    let timed_out = matches!(&error, HttpError::Request(source) if source.is_timeout());
    if timed_out {
        return McpHttpError::Timeout(error);
    }
    let refused = matches!(&error, HttpError::Request(source) if source.is_connect());
    if refused {
        return McpHttpError::Connect(error);
    }
    McpHttpError::Request(error)
}
