use std::sync::Arc;

use http::HeaderMap;
use http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderName, HeaderValue};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::tonic_types::metadata::MetadataMap;
use tonic::Request as TonicRequest;
use tonic::Status;
use tonic::service::Interceptor;

use crate::otel::plumbing::headers::parse_headers;

use super::ExporterError;

pub(super) fn configured_headers(raw_headers: Option<&str>) -> Result<HeaderMap, ExporterError> {
    let mut headers = HeaderMap::new();
    for (name, value) in parse_headers(raw_headers) {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(&value)?,
        );
    }
    Ok(headers)
}

// opentelemetry-otlp merges the OTEL_EXPORTER_OTLP_*_HEADERS variables over with_headers, while LiteLLM reads them only when a spec configures none.
#[derive(Debug)]
pub(super) struct ConfiguredHeadersClient {
    pub(super) inner: Arc<dyn HttpClient>,
    pub(super) headers: HeaderMap,
}

#[async_trait::async_trait]
impl HttpClient for ConfiguredHeadersClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let (mut parts, body) = request.into_parts();
        let mut headers = self.headers.clone();
        for name in [CONTENT_TYPE, CONTENT_ENCODING] {
            if let Some(value) = parts.headers.get(&name) {
                headers.insert(name, value.clone());
            }
        }
        parts.headers = headers;
        self.inner
            .send_bytes(Request::from_parts(parts, body))
            .await
    }
}

// The tonic exporter merges the environment headers after with_metadata, and its custom interceptor runs last.
#[derive(Clone)]
pub(super) struct MetadataInterceptor {
    pub(super) headers: HeaderMap,
}

impl Interceptor for MetadataInterceptor {
    fn call(&mut self, mut request: TonicRequest<()>) -> Result<TonicRequest<()>, Status> {
        *request.metadata_mut() = MetadataMap::from_headers(self.headers.clone());
        Ok(request)
    }
}
