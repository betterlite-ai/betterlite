use std::collections::BTreeMap;

use futures_util::stream::BoxStream;
use http::header::{ACCEPT, CACHE_CONTROL, HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;

use crate::agent_card::{AgentCard, WireValue};
use crate::client::A2AClient;
use crate::client::error::ClientError;
use crate::client::records::core::{CoreSendMessageRequest, CoreStreamEvent};

mod exchange;

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
        let url_text = self.endpoint.to_string();
        let streaming = self.streaming && declares_streaming(&self.agent_card);
        let method = if streaming {
            "SendStreamingMessage"
        } else {
            "SendMessage"
        };
        let body = serde_json::to_vec(&JsonRpcEnvelope {
            jsonrpc: "2.0",
            id: request_id,
            method,
            params: request,
        })
        .map_err(|source| ClientError::Encode {
            url: url_text.clone(),
            source,
        })?;
        let parameters = self
            .call_context
            .as_ref()
            .and_then(|call| call.service_parameters.as_ref());
        let headers = headers(parameters, streaming)?;
        let deadline = self.http_client.limits.read_timeout;
        let http = self.http_client.clone();
        let url = self.endpoint.clone();
        if streaming {
            exchange::streaming_events(http, url, headers, body, deadline).await
        } else {
            exchange::buffered_event(http, url, headers, body, deadline).await
        }
    }
}

pub(crate) fn headers(
    parameters: Option<&BTreeMap<String, String>>,
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
        let header_value = HeaderValue::try_from(value.as_str()).map_err(|source| {
            ClientError::InvalidHeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        headers.insert(header_name, header_value);
    }
    headers
        .entry(HeaderName::from_static("a2a-version"))
        .or_insert(HeaderValue::from_static("1.0"));
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
