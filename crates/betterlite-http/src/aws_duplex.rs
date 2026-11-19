use std::sync::Arc;

use aws_smithy_runtime_api::client::http::{
    HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_runtime_api::http::{Headers, Response, StatusCode};
use aws_smithy_types::body::SdkBody;
use reqwest::{Body, Method};

use crate::proxy::ProxiedClient;

mod body;
mod tests;

use body::{ByteStream, RequestBody, ResponseBody};

// reqwest derives these from the URL and the body itself; a forwarded copy would contradict it
const REBUILT_HEADERS: [&str; 4] = ["connection", "content-length", "host", "transfer-encoding"];

// The Smithy HTTP client the AWS Bedrock Runtime SDK drives, backed by the shared proxied reqwest client
#[derive(Clone, Debug)]
pub struct AwsDuplexClient {
    client: Arc<ProxiedClient>,
}

impl AwsDuplexClient {
    #[must_use]
    pub fn from_proxied(client: &ProxiedClient) -> Self {
        Self {
            client: Arc::new(client.clone()),
        }
    }

    // The request and response bodies both stay streams: the SDK sends and receives events while the exchange is open
    #[must_use]
    pub fn connector(&self) -> SharedHttpConnector {
        SharedHttpConnector::new(AwsDuplexConnector {
            client: Arc::clone(&self.client),
        })
    }
}

impl HttpClient for AwsDuplexClient {
    fn http_connector(
        &self,
        _settings: &HttpConnectorSettings,
        _components: &RuntimeComponents,
    ) -> SharedHttpConnector {
        self.connector()
    }
}

#[derive(Clone, Debug)]
struct AwsDuplexConnector {
    client: Arc<ProxiedClient>,
}

impl HttpConnector for AwsDuplexConnector {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let connector = self.clone();
        HttpConnectorFuture::new(async move { connector.exchange(request).await })
    }
}

impl AwsDuplexConnector {
    async fn exchange(&self, request: HttpRequest) -> Result<HttpResponse, ConnectorError> {
        let built = self.build_request(request)?;
        let response = self
            .client
            .execute(built)
            .await
            .map_err(|source| ConnectorError::io(Box::new(source)))?;
        self.read_response(response)
    }

    fn build_request(&self, request: HttpRequest) -> Result<reqwest::Request, ConnectorError> {
        if self.client.tracker.is_closed() {
            return Err(ConnectorError::io(Box::new(std::io::Error::other(
                "the HTTP client is closed",
            ))));
        }
        let uri = request.uri().to_owned();
        let method = Method::from_bytes(request.method().as_bytes())
            .map_err(|source| ConnectorError::user(Box::new(source)))?;
        let mut builder = self.client.client.request(method, uri.as_str());
        for (name, value) in request.headers().iter() {
            if REBUILT_HEADERS
                .iter()
                .any(|skipped| name.eq_ignore_ascii_case(skipped))
            {
                continue;
            }
            builder = builder.header(name, value);
        }
        let io = self.client.tracker.token();
        let body = Body::wrap(RequestBody {
            body: request.into_body(),
            _io: io,
        });
        builder
            .body(body)
            .build()
            .map_err(|source| ConnectorError::user(Box::new(source)))
    }

    fn read_response(
        &self,
        mut response: reqwest::Response,
    ) -> Result<HttpResponse, ConnectorError> {
        let status = StatusCode::try_from(response.status().as_u16())
            .map_err(|source| ConnectorError::other(Box::new(source), None))?;
        let mut headers = Headers::new();
        let received = std::mem::take(response.headers_mut());
        for (name, value) in &received {
            let text = value
                .to_str()
                .map_err(|source| ConnectorError::other(Box::new(source), None))?;
            headers
                .try_insert(name.as_str().to_owned(), text.to_owned())
                .map_err(|source| ConnectorError::other(Box::new(source), None))?;
        }
        let io = self.client.tracker.token();
        let stream: ByteStream = Box::pin(response.bytes_stream());
        let body = SdkBody::from_body_1_x(ResponseBody::new(stream, io));
        let mut answer = Response::new(status, body);
        *answer.headers_mut() = headers;
        Ok(answer)
    }
}
