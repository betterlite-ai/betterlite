use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Method};
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use sse_stream::Sse;
use tracing::warn;
use url::Url;

use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::McpHttpTransport;
use crate::client::sdk_transport::error::{SdkError, report_failure};
use crate::client::sdk_transport::event_stream::{EventStream, MAX_EVENT_SIZE};
use crate::client::sdk_transport::origin::{legacy_failure, send_within_origin};
use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::stream::{MessageHandler, StreamFailureKind};
use crate::client::sessions::transport::TransportContext;

// the SDK's sse_read_timeout default; LiteLLM never overrides it
const SSE_READ_TIMEOUT: Duration = Duration::from_secs(300);
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
    Message(RxJsonRpcMessage<RoleClient>),
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
    fn enter(&mut self) -> impl Future<Output = Result<LegacySseStreams, SdkError>> + Send {
        let url = self.url.clone();
        let http = Arc::clone(&self.http);
        let handler = self.handler.clone();
        async move {
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
            let response = send_within_origin(&http, request).await?;
            if !(200..=299).contains(&response.status) {
                return Err(legacy_failure(&url, response));
            }
            let mut events = EventStream::open(response.body, MAX_EVENT_SIZE);
            loop {
                let Some(event) = events.next().await else {
                    return Err(SdkError::EndpointMissing);
                };
                match event {
                    Err(error) => return Err(error),
                    Ok(sse) => {
                        if sse.event.as_deref() == Some("endpoint") {
                            let endpoint = endpoint_of(&url, &sse)?;
                            return Ok(LegacySseStreams {
                                endpoint,
                                http,
                                events,
                                handler,
                            });
                        }
                    }
                }
            }
        }
    }
    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<SdkError>>> + Send + 'static {
        async move { Ok(()) }
    }
}
impl LegacySseStreams {
    fn accept(&self, event: Result<Sse, SdkError>) -> Received {
        match event {
            Err(error) => {
                report_failure(&self.handler, StreamFailureKind::Http, error);
                Received::Stop
            }
            Ok(sse) => match sse.event.as_deref() {
                None | Some("message") => {
                    let data = sse.data.as_deref().unwrap_or_default();
                    if data.is_empty() {
                        return Received::Skip;
                    }
                    match serde_json::from_str::<RxJsonRpcMessage<RoleClient>>(data) {
                        Ok(message) => Received::Message(message),
                        Err(source) => {
                            report_failure(
                                &self.handler,
                                StreamFailureKind::ValueError,
                                SdkError::Json(source),
                            );
                            Received::Stop
                        }
                    }
                }
                Some("endpoint") => Received::Skip,
                other => {
                    warn!("Unknown SSE event: {}", other.unwrap_or(""));
                    Received::Skip
                }
            },
        }
    }
}
impl Transport<RoleClient> for LegacySseStreams {
    type Error = SdkError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), SdkError>> + Send + 'static {
        let http = Arc::clone(&self.http);
        let endpoint = self.endpoint.clone();
        async move { post(&http, &endpoint, item).await }
    }
    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        async move {
            loop {
                let Some(event) = self.events.next().await else {
                    return None;
                };
                match self.accept(event) {
                    Received::Message(message) => return Some(message),
                    Received::Skip => {}
                    Received::Stop => return None,
                }
            }
        }
    }
    fn close(&mut self) -> impl Future<Output = Result<(), SdkError>> + Send {
        async move { Ok(()) }
    }
}
async fn post(
    http: &McpHttpTransport,
    endpoint: &Url,
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
    let mut response = send_within_origin(http, request).await?;
    drop(response.body.read_all().await?);
    if !(200..=299).contains(&response.status) {
        return Err(legacy_failure(endpoint, response));
    }
    Ok(())
}
