use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::Object;
use http::header::{HeaderMap, HeaderName, HeaderValue};

use crate::openapi_tools::document::{OpenApiDocument, Operation};
use crate::openapi_tools::request_headers::OpenApiCallContext;
use crate::openapi_tools::tool::{OpenApiTool, OpenApiToolConfig};
use crate::openapi_tools::transport::{OpenApiRequest, OpenApiTransport, OpenApiTransportError};

pub const BASE: &str = "http://upstream.test";

pub fn document(json: &str) -> OpenApiDocument {
    serde_json::from_str(json).unwrap()
}

pub fn operation(json: &str) -> Operation {
    serde_json::from_str(json).unwrap()
}

pub fn arguments(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

pub fn json(text: &str) -> betterlite_prompts::arguments::json::Json {
    serde_json::from_str(text).unwrap()
}

pub fn config(method: &str, path: &str, operation_json: &str) -> OpenApiToolConfig {
    OpenApiToolConfig {
        path: path.to_owned(),
        method: method.to_owned(),
        operation: operation(operation_json),
        base_url: BASE.to_owned(),
        headers: BTreeMap::new(),
        server_label: None,
        relays_upstream_auth: false,
        auth_type: None,
        upstream_token_header: None,
    }
}

pub fn tool(method: &str, path: &str, operation_json: &str) -> OpenApiTool {
    OpenApiTool::new(config(method, path, operation_json))
}

pub fn context() -> OpenApiCallContext {
    OpenApiCallContext::default()
}

pub struct FakeTransport {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    requests: Mutex<Vec<OpenApiRequest>>,
}

impl FakeTransport {
    pub fn replying(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.to_vec(),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.append(
            HeaderName::from_static(name),
            HeaderValue::from_str(value).unwrap(),
        );
        self
    }

    pub fn with_header_bytes(mut self, name: &'static str, value: &[u8]) -> Self {
        self.headers.append(
            HeaderName::from_static(name),
            HeaderValue::from_bytes(value).unwrap(),
        );
        self
    }

    pub fn requests(&self) -> Vec<OpenApiRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn only_request(&self) -> OpenApiRequest {
        let mut requests = self.requests();
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    }
}

impl OpenApiTransport for FakeTransport {
    fn send<'a>(
        &'a self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'a>>
    {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let response = UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        };
        Box::pin(async move { Ok(response) })
    }
}
