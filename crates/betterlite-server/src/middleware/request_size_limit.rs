mod tests;

use std::convert::Infallible;
use std::future::{Future, ready};
use std::ops::Mul as _;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::Response;
use betterlite_config::sections::request_size_limit::Settings;
use futures_util::StreamExt as _;
use futures_util::stream;
use thiserror::Error;
use tower::{Layer, Service};

const BYTES_PER_MB: f64 = 1_048_576.0;

#[derive(Debug, Error)]
pub enum RequestSizeLimitError {
    #[error("the request body is larger than the configured maximum")]
    TooLarge,
    #[error("the request body could not be read")]
    Body {
        #[source]
        source: axum::Error,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SizeLimitLayer {
    pub(crate) max_bytes: u64,
    pub(crate) max_mb: f64,
}

impl SizeLimitLayer {
    // None when LiteLLM's _mb_to_bytes answers None: an unset or non-positive value installs nothing
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Option<Self> {
        let max_mb = settings.max_request_size_mb?;
        if max_mb <= 0.0_f64 {
            return None;
        }
        let bytes = max_mb.mul(BYTES_PER_MB);
        let max_bytes = if bytes.is_finite() {
            format!("{:.0}", bytes.trunc()).parse().unwrap_or(u64::MAX)
        } else {
            u64::MAX
        };
        Some(Self { max_bytes, max_mb })
    }
}

impl<S> Layer<S> for SizeLimitLayer {
    type Service = SizeLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SizeLimitService {
            inner,
            limit: *self,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SizeLimitService<S> {
    inner: S,
    limit: SizeLimitLayer,
}

impl<S> Service<Request<Body>> for SizeLimitService<S>
where
    S: Service<Request<Body>, Response = Response, Error = Infallible>,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let limit = self.limit;
        if let Some(length) = content_length(req.headers())
            && length > u128::from(limit.max_bytes)
        {
            return Box::pin(ready(Ok::<Response, Infallible>(too_large(limit.max_mb))));
        }
        let (parts, body) = req.into_parts();
        let exceeded = Arc::new(AtomicBool::new(false));
        let counted = counted_body(body, limit.max_bytes, Arc::clone(&exceeded));
        let serving = self.inner.call(Request::from_parts(parts, counted));
        Box::pin(async move {
            let response: Response = serving.await?;
            Ok(if exceeded.load(Ordering::Acquire) {
                too_large(limit.max_mb)
            } else {
                response
            })
        })
    }
}

// LiteLLM counts the ASGI body messages, which ends in a 413 once the running total passes the limit
fn counted_body(body: Body, limit: u64, exceeded: Arc<AtomicBool>) -> Body {
    let body_stream = body.into_data_stream();
    let counted = stream::unfold((body_stream, 0_u64), move |(mut stream, total)| {
        let flag = Arc::clone(&exceeded);
        async move {
            let next = stream.next().await?;
            let (result, running) = match next {
                Ok(chunk) => count_chunk(chunk, total, limit, &flag),
                Err(source) => (Err(RequestSizeLimitError::Body { source }), total),
            };
            Some((result, (stream, running)))
        }
    });
    Body::from_stream(counted)
}

fn count_chunk(
    chunk: Bytes,
    total: u64,
    limit: u64,
    exceeded: &AtomicBool,
) -> (Result<Bytes, RequestSizeLimitError>, u64) {
    let counted = total.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
    if counted > limit {
        exceeded.store(true, Ordering::Release);
        return (Err(RequestSizeLimitError::TooLarge), counted);
    }
    (Ok(chunk), counted)
}

// A dict built from the header list keeps the last duplicate; only a plain decimal integer is a length
fn content_length(headers: &HeaderMap) -> Option<u128> {
    let value = headers.get_all(header::CONTENT_LENGTH).iter().next_back()?;
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(bytes.iter().fold(0_u128, |total, digit| {
        total
            .saturating_mul(10)
            .saturating_add(u128::from(digit.saturating_sub(b'0')))
    }))
}

// LiteLLM answers a plain {"error": ...}, so the OpenAI envelope would change what a client sees
fn too_large(max_mb: f64) -> Response {
    let body = format!("{{\"error\":\"Request size is too large. Max size is {max_mb} MB\"}}");
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
