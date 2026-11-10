use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::bytez_wire::error::BytezError;
use betterlite_openai::bytez_wire::request::BytezRequestBody;
use betterlite_prompts::arguments::json::Json;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use futures_util::StreamExt as _;
use http::{HeaderMap, Method};
use tokio::time::timeout;
use url::Url;

use self::error::BytezStreamError;
use self::stream::{BytezCustomStream, text_fragments};
use self::transport::{BytezChunks, BytezStreamTransport, ClientError, StreamHead};

pub mod error;
pub mod stream;
pub mod transport;

mod tests;

// httpx applies timeout=300 to opening the response and to each read, never to the whole stream
pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(300);

// the pooled transport's own body cap, repeated so the error body read is bounded without it
pub const MAX_BODY_BYTES: usize = 0x0100_0000;

// one call serves both LiteLLM custom stream wrappers: kind picks the sync or async client
pub struct BytezStreamCall<'request> {
    pub model: String,
    pub provider: String,
    pub url: String,
    pub headers: HeaderMap,
    pub body: BytezRequestBody,
    pub kind: ClientKind,
    pub supplied: Option<Arc<dyn BytezStreamTransport>>,
    pub default_client: Arc<dyn BytezStreamTransport>,
    pub repeated_chunk_limit: usize,
    pub redactor: &'request Redactor,
}

impl<'request> BytezStreamCall<'request> {
    pub async fn open(mut self) -> Result<BytezCustomStream<'request>, BytezStreamError> {
        self.body.stream = Json::Bool(true);
        let body =
            serde_json::to_vec(&self.body).map_err(|source| BytezStreamError::Encode { source })?;
        let url = Url::parse(&self.url).map_err(|source| BytezStreamError::Url { source })?;
        let supplied = self.supplied.filter(|client| client.kind() == self.kind);
        let client = supplied.unwrap_or(self.default_client);
        let request = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: self.headers,
            body: RequestBody::Json(body),
            deadline: TransportLimits::DEFAULT.read_timeout,
        };
        let head = open_head(client.as_ref(), request).await?;
        if head.status != 200 {
            let message = error_body(head.chunks).await?;
            return Err(BytezStreamError::Api(BytezError {
                status_code: head.status,
                message,
            }));
        }
        Ok(BytezCustomStream {
            shaper: ChunkShaper::new(self.model, Some(self.provider), false, JsonObject::new()),
            repeated_chunk_limit: self.repeated_chunk_limit,
            fragments: text_fragments(head.chunks),
            redactor: self.redactor,
        })
    }
}

// except httpx.HTTPStatusError becomes BytezError with the response's status and text
async fn open_head(
    client: &dyn BytezStreamTransport,
    request: OutboundRequest,
) -> Result<StreamHead, BytezStreamError> {
    let opened = timeout(STREAMING_TIMEOUT, client.open(request))
        .await
        .map_err(|_elapsed| BytezStreamError::Timeout)?;
    match opened {
        Ok(head) => Ok(head),
        Err(ClientError::Status {
            status_code,
            message,
        }) => Err(BytezStreamError::Api(BytezError {
            status_code,
            message,
        })),
        Err(failure @ ClientError::Transport { .. }) => Err(BytezStreamError::Client(failure)),
    }
}

async fn error_body(mut chunks: BytezChunks) -> Result<String, BytezStreamError> {
    let mut collected: Vec<u8> = Vec::new();
    while let Some(chunk) = timeout(STREAMING_TIMEOUT, chunks.next())
        .await
        .map_err(|_elapsed| BytezStreamError::Timeout)?
    {
        let bytes = chunk.map_err(|source| BytezStreamError::Body { source })?;
        collected
            .len()
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_BODY_BYTES)
            .ok_or(BytezStreamError::TooLarge {
                limit: MAX_BODY_BYTES,
            })?;
        collected.extend_from_slice(&bytes);
    }
    String::from_utf8(collected).map_err(|source| BytezStreamError::InvalidUtf8 {
        source: source.utf8_error(),
    })
}
