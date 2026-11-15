use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt as _;
use http::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Method};
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use sse_stream::Sse;
use tracing::warn;
use url::Url;

use crate::client::cleanup::http_client::HttpResponse;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::McpHttpTransport;
use crate::client::sdk_transport::error::{SdkError, report_failure};
use crate::client::sdk_transport::event_stream::{EventStream, MAX_EVENT_SIZE, open_stream};
use crate::client::sdk_transport::origin::{legacy_failure, send_within_origin};
use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::stream::{MessageHandler, StreamFailureKind};
use crate::client::sessions::transport::TransportContext;
// the SDK's sse_read_timeout default; LiteLLM never overrides it
pub(super) const SSE_READ_TIMEOUT: Duration = Duration::from_secs(300);
pub struct LegacySse {
    pub url: Url,
    pub http: Arc<McpHttpTransport>,
    pub handler: MessageHandler<SdkError>,
}
pub struct LegacySseStreams {
    endpoint: Url,
    http: Arc<McpHttpTransport>,
    events: EventStream,
    handler: MessageHandler<SdkError>,
}
enum Received {
    Message(Box<RxJsonRpcMessage<RoleClient>>),
    Skip,
    Stop,
}
fn endpoint_of(base: &Url, sse: &Sse) -> Result<Url, SdkError> {
    let data = sse.data.as_deref().unwrap_or_default();
    let endpoint = base.join(data).map_err(SdkError::EndpointUrl)?;
    let same_origin = endpoint.scheme() == base.scheme()
        && endpoint.host_str() == base.host_str()
        && endpoint.port() == base.port();
    if same_origin {
        return Ok(endpoint);
    }
    Err(SdkError::OriginMismatch {
        endpoint_url: endpoint.to_string(),
    })
}
impl TransportContext for LegacySse {
    type Streams = LegacySseStreams;
    type Error = SdkError;
    async fn enter(&mut self) -> Result<LegacySseStreams, SdkError> {
        open_streams(&self.url, &self.http, self.handler.clone()).await
    }
    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        drop(self);
        ready(Ok(()))
    }
}
async fn open_streams(
    url: &Url,
    http: &Arc<McpHttpTransport>,
    handler: MessageHandler<SdkError>,
) -> Result<LegacySseStreams, SdkError> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let request = McpHttpRequest {
        method: Method::GET,
        url: url.clone(),
        headers,
        body: Bytes::new(),
        read_timeout: Some(SSE_READ_TIMEOUT),
    };
    let response = send_within_origin(http, request).await?;
    if !(200..=299).contains(&response.status) {
        return Err(legacy_failure(url, response));
    }
    let HttpResponse {
        headers: response_headers,
        body,
        ..
    } = response;
    let mut events = open_stream(body, MAX_EVENT_SIZE, &response_headers);
    loop {
        let sse = match events.next().await {
            None => return Err(SdkError::EndpointMissing),
            Some(Err(error)) => return Err(error),
            Some(Ok(sse)) => sse,
        };
        if sse.event.as_deref() == Some("endpoint") {
            let endpoint = endpoint_of(url, &sse)?;
            return Ok(LegacySseStreams {
                endpoint,
                http: Arc::clone(http),
                events,
                handler,
            });
        }
    }
}
impl LegacySseStreams {
    fn accept(&self, event: Result<Sse, SdkError>) -> Received {
        let sse = match event {
            Err(error) => {
                report_failure(&self.handler, StreamFailureKind::Http, error);
                return Received::Stop;
            }
            Ok(sse) => sse,
        };
        match sse.event.as_deref() {
            None | Some("" | "message") => self.message(&sse),
            Some("endpoint") => Received::Skip,
            Some(other) => {
                warn!("Unknown SSE event: {other}");
                Received::Skip
            }
        }
    }
    fn message(&self, sse: &Sse) -> Received {
        let data = sse.data.as_deref().unwrap_or_default();
        if data.is_empty() {
            return Received::Skip;
        }
        match serde_json::from_str::<RxJsonRpcMessage<RoleClient>>(data) {
            Ok(message) => Received::Message(Box::new(message)),
            Err(source) => {
                let error = SdkError::Json(source);
                report_failure(&self.handler, StreamFailureKind::ValueError, error);
                Received::Stop
            }
        }
    }
}
impl Transport<RoleClient> for LegacySseStreams {
    type Error = SdkError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        post(Arc::clone(&self.http), self.endpoint.clone(), item)
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        loop {
            let event = self.events.next().await?;
            match self.accept(event) {
                Received::Message(message) => return Some(*message),
                Received::Skip => {}
                Received::Stop => return None,
            }
        }
    }
    fn close(&mut self) -> impl Future<Output = Result<(), SdkError>> + Send {
        ready(Ok(()))
    }
}
async fn post(
    http: Arc<McpHttpTransport>,
    endpoint: Url,
    item: TxJsonRpcMessage<RoleClient>,
) -> Result<(), SdkError> {
    let body = serde_json::to_vec(&item).map_err(SdkError::Json)?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let request = McpHttpRequest {
        method: Method::POST,
        url: endpoint.clone(),
        headers,
        body: Bytes::from(body),
        read_timeout: None,
    };
    let mut response = send_within_origin(&http, request).await?;
    drop(response.body.read_all().await?);
    if !(200..=299).contains(&response.status) {
        return Err(legacy_failure(&endpoint, response));
    }
    Ok(())
}
