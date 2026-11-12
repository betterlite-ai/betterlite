mod tests;

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode};
use axum::response::Response;
use tower::{Layer, Service};

const STATIC_HEADERS: [(&str, &str); 3] = [
    ("x-frame-options", "DENY"),
    ("content-security-policy", "frame-ancestors 'none'"),
    ("x-content-type-options", "nosniff"),
];
const HSTS_HEADER: (&str, &str) = (
    "strict-transport-security",
    "max-age=31536000; includeSubDomains",
);

#[derive(Debug, Clone, Copy, Default)]
pub struct SecurityHeadersLayer {
    pub enable_hsts: bool,
}

#[derive(Debug, Clone)]
pub struct SecurityHeadersService<S> {
    inner: S,
    enable_hsts: bool,
}

impl<S> Layer<S> for SecurityHeadersLayer {
    type Service = SecurityHeadersService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SecurityHeadersService {
            inner,
            enable_hsts: self.enable_hsts,
        }
    }
}

impl<S> Service<Request<Body>> for SecurityHeadersService<S>
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
        let enable_hsts = self.enable_hsts;
        let serving = self.inner.call(req);
        Box::pin(async move {
            let mut response: Response = serving.await?;
            apply_headers(&mut response, enable_hsts);
            Ok(response)
        })
    }
}

fn apply_headers(response: &mut Response, enable_hsts: bool) {
    // a websocket scope never reaches Starlette's HTTP branch, and its handshake is a 101
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        return;
    }
    let mut applied = STATIC_HEADERS.to_vec();
    if enable_hsts {
        applied.push(HSTS_HEADER);
    }
    let headers = response.headers_mut();
    // Starlette's setdefault keeps a value the route set itself
    for (name, value) in applied {
        headers
            .entry(HeaderName::from_static(name))
            .or_insert(HeaderValue::from_static(value));
    }
}
