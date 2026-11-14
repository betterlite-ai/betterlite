use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::messages_sse_events::stream::{
    MessagesProviderStream, MessagesStreamHiddenParams, MessagesStreamingResponse,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::stream::Lease;
use betterlite_models::decisions::Json;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use reqwest::Response;
use serde::Serialize;
use url::Url;

use super::MessagesHeaderStreamCall;
use super::error::SdkHttpError;
use super::result::{HeaderStreamRun, SdkResult};
use super::vector_query_embedding::outbound_client;

const STREAM_DEADLINE: Duration = Duration::from_secs(120);
const MAX_STREAMED_BYTES: usize = 0x0100_0000;

struct HttpChunks {
    response: Response,
    _lease: Lease,
    buffered: Option<Json>,
    closed: bool,
}

#[async_trait]
impl MessagesProviderStream for HttpChunks {
    type Error = SdkHttpError;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SdkHttpError> {
        self.response
            .chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|source| SdkHttpError::Stream(Box::new(HttpError::Request(source))))
    }

    async fn aclose(&mut self) -> Option<Result<(), SdkHttpError>> {
        self.closed = true;
        Some(Ok(()))
    }

    fn buffered_provider_output(&self) -> Option<Json> {
        self.buffered.clone()
    }
}

pub(crate) async fn send(
    call: &MessagesHeaderStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let request = outbound(api_base, &call.body)?;
    let client = outbound_client(env)?;
    let sent = client
        .send(request)
        .await
        .map_err(|error| SdkHttpError::Transport(Box::new(error)))?;
    let hidden_params = MessagesStreamHiddenParams::from(&projected_headers(&sent.response)?);
    let stream = HttpChunks {
        response: sent.response,
        _lease: sent.lease,
        buffered: call.buffered_provider_output.clone(),
        closed: false,
    };
    let mut view = MessagesStreamingResponse {
        completion_stream: stream,
        hidden_params,
    };
    let provider_bytes = drain(&mut view).await?;
    if let Some(closed) = view.aclose().await {
        closed?;
    }
    let has_buffered_provider_output = view.has_buffered_provider_output();
    let closed = view.completion_stream.closed;
    Ok(SdkResult::MessagesHeaderStream {
        value: HeaderStreamRun {
            hidden_params: view.hidden_params,
            provider_bytes,
            has_buffered_provider_output,
            closed,
        },
    })
}

pub(super) fn outbound<B>(api_base: &str, body: &B) -> Result<OutboundRequest, SdkHttpError>
where
    B: Serialize + ?Sized,
{
    let url = Url::parse(&format!("{api_base}/messages"))
        .map_err(|source| SdkHttpError::Url(Box::new(source)))?;
    let encoded = serde_json::to_vec(body).map_err(SdkHttpError::Serialize)?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(encoded),
        deadline: STREAM_DEADLINE,
    })
}

async fn drain(view: &mut MessagesStreamingResponse<HttpChunks>) -> Result<Vec<u8>, SdkHttpError> {
    let mut provider_bytes = Vec::new();
    while let Some(chunk) = view.next_chunk().await? {
        if provider_bytes.len().saturating_add(chunk.len()) > MAX_STREAMED_BYTES {
            return Err(SdkHttpError::StreamTooLarge {
                limit: MAX_STREAMED_BYTES,
            });
        }
        provider_bytes.extend_from_slice(&chunk);
    }
    Ok(provider_bytes)
}

fn projected_headers(response: &Response) -> Result<IndexMap<String, String>, SdkHttpError> {
    let mut headers = IndexMap::with_capacity(response.headers().len());
    for (name, value) in response.headers() {
        let text = value
            .to_str()
            .map_err(|_source| SdkHttpError::HeaderValue(name.as_str().to_owned()))?;
        headers.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(headers)
}
