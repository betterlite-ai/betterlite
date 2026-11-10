use std::sync::Arc;
use std::time::Duration;

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
use url::Url;

use self::error::BytezStreamError;
use self::stream::{BytezCustomStream, text_fragments};
use self::transport::{BytezChunks, BytezStreamTransport};

pub mod error;
pub mod stream;
pub mod transport;

mod tests;

// LiteLLM gives a Bytez stream five minutes because the model may still be loading
pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(300);

// the pooled transport's own body cap, repeated so the error body read is bounded without it
pub const MAX_BODY_BYTES: usize = 0x0100_0000;

// one call serves get_sync_custom_stream_wrapper and get_async_custom_stream_wrapper: kind picks the client
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
        let head = client
            .open(OutboundRequest {
                method: Method::POST,
                url,
                query: Vec::new(),
                headers: self.headers,
                body: RequestBody::Json(body),
                deadline: STREAMING_TIMEOUT,
            })
            .await?;
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

async fn error_body(mut chunks: BytezChunks) -> Result<String, BytezStreamError> {
    let mut collected: Vec<u8> = Vec::new();
    while let Some(chunk) = chunks.next().await {
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
