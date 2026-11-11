mod tests;

use std::convert::Infallible;
use std::future::Future;
use std::io::{self, Write as _};
use std::mem;
use std::pin::Pin;
use std::str;
use std::task::{Context, Poll};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use flate2::Compression;
use flate2::write::GzEncoder;
use http_body::Body as _;
use tokio::task::JoinSet;
use tower::{Layer, Service};

const MINIMUM_SIZE_BYTES: u64 = 500;
const OFF_LOOP_SIZE_BYTES: u64 = 1024 * 1024;
const COMPRESS_LEVEL: u32 = 6;
const NO_TRANSFORM: &[u8] = b"no-transform";

// LiteLLM reads the header through Starlette's latin-1 Headers, so non-UTF-8 bytes never fail it
#[must_use]
pub fn accepts_gzip(raw: &[u8]) -> bool {
    let mut gzip = None;
    let mut x_gzip = None;
    let mut any = None;
    for part in raw.split(|byte| *byte == b',') {
        if part.trim_ascii().is_empty() {
            continue;
        }
        let (coding, parameters) = part.iter().position(|byte| *byte == b';').map_or_else(
            || (part, &[][..]),
            |index| {
                (
                    part.get(..index).unwrap_or_default(),
                    part.get(index.saturating_add(1)..).unwrap_or_default(),
                )
            },
        );
        let name = coding.trim_ascii();
        let weight = q_weight(parameters);
        match name {
            b"*" => any = Some(weight),
            _ if name.eq_ignore_ascii_case(b"gzip") => gzip = Some(weight),
            _ if name.eq_ignore_ascii_case(b"x-gzip") => x_gzip = Some(weight),
            _ => {}
        }
    }
    // a present coding wins over its fallbacks even at weight 0, and the last duplicate wins
    gzip.or(x_gzip).or(any).unwrap_or(0.0_f64) > 0.0_f64
}

// the q value comes from the first parameter that starts with "q=", and a non-number counts as 0
fn q_weight(parameters: &[u8]) -> f64 {
    let Some(parameter) = parameters
        .split(|byte| *byte == b';')
        .map(<[u8]>::trim_ascii)
        .find(|parameter| {
            parameter
                .get(..2)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"q="))
        })
    else {
        return 1.0_f64;
    };
    parameter
        .get(2..)
        .and_then(|value| str::from_utf8(value).ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .unwrap_or(0.0_f64)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GzipBufferedLayer;

#[derive(Debug, Clone)]
pub struct GzipBufferedService<S> {
    inner: S,
}

impl<S> Layer<S> for GzipBufferedLayer {
    type Service = GzipBufferedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GzipBufferedService { inner }
    }
}

impl<S> Service<Request<Body>> for GzipBufferedService<S>
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
        let accepted = req
            .headers()
            .get(header::ACCEPT_ENCODING)
            .is_some_and(|value| accepts_gzip(value.as_bytes()));
        let method = req.method().clone();
        let serving = self.inner.call(req);
        Box::pin(async move {
            let response: Response = serving.await?;
            Ok(respond(response, accepted, &method).await)
        })
    }
}

// Starlette's responder holds the response head until a complete body arrives, so only a body in hand is compressed
async fn respond(mut response: Response, accepted: bool, method: &Method) -> Response {
    if *method == Method::HEAD || response.status() == StatusCode::SWITCHING_PROTOCOLS {
        return response;
    }
    let Some(size) = response.body().size_hint().exact() else {
        return response;
    };
    if size < MINIMUM_SIZE_BYTES
        || response.status() == StatusCode::PARTIAL_CONTENT
        || response.headers().contains_key(header::CONTENT_ENCODING)
        || response.headers().contains_key(header::ETAG)
        || no_transform(response.headers())
    {
        return response;
    }
    append_vary(response.headers_mut());
    if !accepted {
        return response;
    }
    let Some(compressed) = compress_body(&mut response, size).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(length) = HeaderValue::from_str(&compressed.len().to_string()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    headers.insert(header::CONTENT_LENGTH, length);
    *response.body_mut() = Body::from(compressed);
    response
}

fn no_transform(headers: &HeaderMap) -> bool {
    headers.get(header::CACHE_CONTROL).is_some_and(|value| {
        value
            .as_bytes()
            .to_ascii_lowercase()
            .windows(NO_TRANSFORM.len())
            .any(|window| window == NO_TRANSFORM)
    })
}

// Starlette's add_vary_header reads the first Vary value and its __setitem__ drops any duplicate
fn append_vary(headers: &mut HeaderMap) {
    let value = headers.get(header::VARY).map_or_else(
        || b"Accept-Encoding".to_vec(),
        |existing| {
            let mut joined = existing.as_bytes().to_vec();
            joined.extend_from_slice(b", Accept-Encoding");
            joined
        },
    );
    if let Ok(vary) = HeaderValue::from_bytes(&value) {
        headers.insert(header::VARY, vary);
    }
}

async fn compress_body(response: &mut Response, size: u64) -> Option<Vec<u8>> {
    let body = mem::replace(response.body_mut(), Body::empty());
    let limit = usize::try_from(size).ok()?;
    let bytes = to_bytes(body, limit).await.ok()?;
    if size >= OFF_LOOP_SIZE_BYTES {
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || gzip_bytes(&bytes));
        tasks.join_next().await?.ok()?.ok()
    } else {
        gzip_bytes(&bytes).ok()
    }
}

fn gzip_bytes(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(COMPRESS_LEVEL));
    encoder.write_all(bytes)?;
    encoder.finish()
}
