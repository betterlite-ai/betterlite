use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use futures_util::stream::BoxStream;
use http::Method;
use http::header::{ACCEPT, CACHE_CONTROL, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::agent_card::{AgentCard, WireValue};
use crate::client::A2AClient;
use crate::client::error::ClientError;
use crate::client::records::core::{CoreSendMessageRequest, CoreStreamEvent};
use crate::client::records::{
    A2A_VERSION, A2A_VERSION_HEADER, JSONRPC_VERSION, SEND_MESSAGE_METHOD,
    SEND_STREAMING_MESSAGE_METHOD,
};

mod exchange;

// httpx limits connect and every read separately; the pooled client carries those limits, so a request has no total deadline
const NO_TOTAL_DEADLINE: Duration = Duration::from_hours(8_760);

pub type EventStream = BoxStream<'static, Result<CoreStreamEvent, ClientError>>;

#[derive(Serialize)]
struct JsonRpcEnvelope<'request> {
    jsonrpc: &'static str,
    id: &'request str,
    method: &'static str,
    params: &'request CoreSendMessageRequest,
}

impl A2AClient {
    pub async fn send_message(
        &self,
        request: &CoreSendMessageRequest,
        request_id: &str,
    ) -> Result<EventStream, ClientError> {
        let streaming = self.streaming && declares_streaming(&self.agent_card);
        let method = if streaming {
            SEND_STREAMING_MESSAGE_METHOD
        } else {
            SEND_MESSAGE_METHOD
        };
        let body = envelope_body(request, request_id, method, self.endpoint.as_str())?;
        let parameters = self
            .call_context
            .as_ref()
            .and_then(|call| call.service_parameters.as_ref());
        let mut headers = headers(parameters, streaming)?;
        headers
            .entry(HeaderName::from_static(A2A_VERSION_HEADER))
            .or_insert(HeaderValue::from_static(A2A_VERSION));
        let timeout = self.timeout;
        let http = self.http_client.clone();
        let outbound = OutboundRequest {
            method: Method::POST,
            url: self.endpoint.clone(),
            query: Vec::new(),
            headers,
            body: RequestBody::Json(body),
            deadline: NO_TOTAL_DEADLINE,
        };
        if streaming {
            exchange::streaming_events(http, outbound, timeout).await
        } else {
            exchange::buffered_event(http, outbound, timeout).await
        }
    }
}

fn envelope_body(
    request: &CoreSendMessageRequest,
    request_id: &str,
    method: &'static str,
    url: &str,
) -> Result<Vec<u8>, ClientError> {
    serde_json::to_vec(&JsonRpcEnvelope {
        jsonrpc: JSONRPC_VERSION,
        id: request_id,
        method,
        params: request,
    })
    .map_err(|source| ClientError::Encode {
        url: url.to_owned(),
        source,
    })
}

pub(crate) fn headers(
    parameters: Option<&BTreeMap<String, SecretString>>,
    streaming: bool,
) -> Result<HeaderMap, ClientError> {
    let mut headers = HeaderMap::new();
    for (name, value) in parameters.into_iter().flatten() {
        let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
            ClientError::InvalidHeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value = HeaderValue::try_from(value.expose_secret()).map_err(|source| {
            ClientError::InvalidHeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        headers.append(header_name, header_value);
    }
    if streaming {
        headers
            .entry(ACCEPT)
            .or_insert(HeaderValue::from_static("text/event-stream"));
        headers
            .entry(CACHE_CONTROL)
            .or_insert(HeaderValue::from_static("no-store"));
    }
    Ok(headers)
}

fn declares_streaming(card: &AgentCard) -> bool {
    let Some(Some(WireValue::Typed(capabilities))) = card.capabilities.as_ref() else {
        return false;
    };
    match capabilities.streaming.as_ref().and_then(Option::as_ref) {
        Some(WireValue::Typed(value)) => *value,
        Some(WireValue::Passthrough(_)) | None => false,
    }
}
