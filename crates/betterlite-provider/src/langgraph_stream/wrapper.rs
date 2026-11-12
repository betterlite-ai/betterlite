use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::aleph_alpha::lines::{LineDecoder, decode_text};
use betterlite_openai::assistants::client::ClientKind;
use betterlite_prompts::message_content::Message;
use futures_util::StreamExt as _;
use http::{HeaderMap, Method};
use tokio::time::timeout;
use url::Url;

use super::error::LangGraphStreamError;
use super::stream::{LangGraphSseStream, LineReader};
use super::{
    Chunks, FIRST_STREAM_RESPONSE, IdSource, MAX_BODY_BYTES, MAX_LINE_BYTES, OK_STATUS,
    STREAMING_TIMEOUT,
};
use crate::langgraph_chat::error::get_error_class;
use crate::langgraph_chat::records::LangGraphRequest;

mod tests;

pub struct StreamHead {
    pub status: u16,
    pub chunks: Chunks,
}

#[async_trait]
pub trait StreamTransport: Send + Sync {
    fn kind(&self) -> ClientKind;

    async fn post(&self, request: OutboundRequest) -> Result<StreamHead, TransportError>;
}

// betterlite has one pooled client and it reads asynchronously, so a sync call replaces it with its default
#[async_trait]
impl StreamTransport for ProxiedClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Async
    }

    async fn post(&self, request: OutboundRequest) -> Result<StreamHead, TransportError> {
        let response = self.send_following(request).await?;
        let status = response.response.status().as_u16();
        Ok(StreamHead {
            status,
            chunks: response.body_stream(),
        })
    }
}

pub struct StreamRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub url: &'request str,
    pub headers: HeaderMap,
    pub data: &'request LangGraphRequest,
    pub messages: &'request [Message],
    pub kind: ClientKind,
}

pub struct StreamDeps<'deps> {
    pub supplied: Option<Arc<dyn StreamTransport>>,
    pub default_client: Arc<dyn StreamTransport>,
    pub ids: Arc<dyn IdSource>,
    pub logger: &'deps dyn StreamLogger,
}

pub struct StreamPostCall<'call> {
    pub input: &'call [Message],
    pub api_key: &'call str,
    pub original_response: &'static str,
    pub complete_input_dict: &'call LangGraphRequest,
}

pub trait StreamLogger: Send + Sync {
    fn post_call(&self, call: StreamPostCall<'_>);
}

pub struct LangGraphStream {
    pub model: String,
    pub provider: String,
    pub iterator: LangGraphSseStream,
}

pub async fn open(
    request: &StreamRequest<'_>,
    deps: &StreamDeps<'_>,
) -> Result<LangGraphStream, LangGraphStreamError> {
    let client = client_for(deps, request.kind);
    let head = client
        .post(outbound(request)?)
        .await
        .map_err(|source| LangGraphStreamError::Transport { source })?;
    if head.status != OK_STATUS {
        let message = error_message(head.chunks).await?;
        return Err(LangGraphStreamError::Provider(get_error_class(
            &message,
            head.status,
        )));
    }
    let iterator = LangGraphSseStream {
        model: request.model.to_owned(),
        ids: Arc::clone(&deps.ids),
        lines: Some(LineReader {
            chunks: head.chunks,
            decoder: LineDecoder::new(MAX_LINE_BYTES),
            lines: VecDeque::new(),
            done: false,
        }),
        finished: false,
    };
    deps.logger.post_call(StreamPostCall {
        input: request.messages,
        api_key: "",
        original_response: FIRST_STREAM_RESPONSE,
        complete_input_dict: request.data,
    });
    Ok(LangGraphStream {
        model: request.model.to_owned(),
        provider: request.provider.to_owned(),
        iterator,
    })
}

fn client_for(deps: &StreamDeps<'_>, kind: ClientKind) -> Arc<dyn StreamTransport> {
    match &deps.supplied {
        Some(supplied) if supplied.kind() == kind => Arc::clone(supplied),
        Some(_) | None => Arc::clone(&deps.default_client),
    }
}

fn outbound(request: &StreamRequest<'_>) -> Result<OutboundRequest, LangGraphStreamError> {
    let url = Url::parse(request.url).map_err(|source| LangGraphStreamError::Url { source })?;
    let body = serde_json::to_vec(request.data)
        .map_err(|source| LangGraphStreamError::Encode { source })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: request.headers.clone(),
        body: RequestBody::Json(body),
        deadline: TransportLimits::DEFAULT.read_timeout,
    })
}

async fn error_message(mut chunks: Chunks) -> Result<String, LangGraphStreamError> {
    let mut body: Vec<u8> = Vec::new();
    loop {
        let next = timeout(STREAMING_TIMEOUT, chunks.next())
            .await
            .map_err(|_elapsed| LangGraphStreamError::Timeout)?;
        let Some(frame) = next else {
            return Ok(decode_text(&body));
        };
        let bytes = frame.map_err(|source| LangGraphStreamError::Body { source })?;
        if body
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > MAX_BODY_BYTES)
        {
            return Err(LangGraphStreamError::TooLarge {
                limit: MAX_BODY_BYTES,
            });
        }
        body.extend_from_slice(&bytes);
    }
}
