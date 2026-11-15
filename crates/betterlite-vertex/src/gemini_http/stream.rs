use std::collections::{BTreeMap, VecDeque};
use std::mem;
use std::sync::Arc;

use betterlite_openai::aleph_alpha::lines::{LineDecoder, decode_text};
use futures_util::StreamExt as _;
use http::header::HeaderMap;
use secrecy::SecretString;
use uuid::Uuid;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::gemini_stream::chunk::StreamChunk;
use crate::gemini_stream::{GeminiStreamDecoder, StreamOptions};
use crate::vertex_embedding_call::post::ClientTimeout;

use super::error::{GeminiHttpError, GeminiStreamError, get_error_class, translate_exception_str};
use super::post::{Chunks, GeminiPost, OpenedReply, PostRequest};

// what a call carries before its first chunk is asked for
pub struct StreamSetup {
    pub url: String,
    pub headers: BTreeMap<String, SecretString>,
    pub body: Vec<u8>,
    pub model: String,
    pub supplied: Option<Arc<dyn GeminiPost>>,
    pub pooled: Arc<dyn GeminiPost>,
    pub ids: Box<dyn FnMut() -> Uuid + Send>,
}

pub struct StreamCall<'call> {
    pub url: &'call str,
    pub headers: &'call BTreeMap<String, SecretString>,
    pub body: &'call [u8],
    pub model: &'call str,
}

// LiteLLM's CustomStreamWrapper around make_call: nothing is posted until the first chunk is consumed
pub struct GeminiHttpStream {
    state: StreamState,
    options: StreamOptions,
}

enum StreamState {
    Pending(Box<StreamSetup>),
    Reading(Box<Reading>),
    Finished,
}

impl GeminiHttpStream {
    #[must_use]
    pub fn new(setup: StreamSetup, options: StreamOptions) -> Self {
        Self {
            state: StreamState::Pending(Box::new(setup)),
            options,
        }
    }

    pub async fn next_chunk(&mut self) -> Option<Result<StreamChunk, GeminiStreamError>> {
        if let StreamState::Reading(reading) = &mut self.state {
            return reading.next().await;
        }
        let StreamState::Pending(setup) = mem::replace(&mut self.state, StreamState::Finished)
        else {
            return None;
        };
        let call = StreamCall {
            url: &setup.url,
            headers: &setup.headers,
            body: &setup.body,
            model: &setup.model,
        };
        let opened = make_call(setup.supplied.as_deref(), setup.pooled.as_ref(), &call).await;
        match opened {
            Ok(reply) => {
                let mut reading = Reading::new(*setup, reply, self.options);
                let next = reading.next().await;
                self.state = StreamState::Reading(Box::new(reading));
                next
            }
            Err(error) => Some(Err(error)),
        }
    }
}

struct Reading {
    chunks: Chunks,
    lines: LineDecoder,
    pending: VecDeque<String>,
    decoder: GeminiStreamDecoder,
    ended: bool,
    error: Option<GeminiStreamError>,
    done: bool,
}

impl Reading {
    fn new(setup: StreamSetup, opened: OpenedReply, options: StreamOptions) -> Self {
        Self {
            chunks: opened.body,
            lines: LineDecoder::new(MAX_RESPONSE_BYTES),
            pending: VecDeque::new(),
            decoder: GeminiStreamDecoder::new(options, &opened.headers, setup.ids),
            ended: false,
            error: None,
            done: false,
        }
    }

    // the queued lines go out before the failure that arrived with them, as httpx's iter_lines does
    async fn next(&mut self) -> Option<Result<StreamChunk, GeminiStreamError>> {
        if self.done {
            return None;
        }
        loop {
            if let Some(line) = self.pending.pop_front() {
                match self.decoder.push(line.as_bytes()) {
                    Ok(None) => continue,
                    Ok(Some(chunk)) => return Some(Ok(chunk)),
                    Err(source) => return self.stop(GeminiStreamError::Chunk(source)),
                }
            }
            if let Some(error) = self.error.take() {
                return self.stop(error);
            }
            if self.ended {
                return self.finish();
            }
            match self.chunks.next().await {
                Some(Ok(bytes)) => self.absorb(&bytes),
                Some(Err(source)) => self.error = Some(GeminiStreamError::body_error(source)),
                None => self.end(),
            }
        }
    }

    // a failure ends the stream once the caller has seen it
    const fn stop(
        &mut self,
        error: GeminiStreamError,
    ) -> Option<Result<StreamChunk, GeminiStreamError>> {
        self.done = true;
        Some(Err(error))
    }

    // the decoder's last chunk comes out once the body ended, and the stream stays finished after it
    fn finish(&mut self) -> Option<Result<StreamChunk, GeminiStreamError>> {
        self.done = true;
        match self.decoder.finish() {
            Ok(None) => None,
            Ok(Some(chunk)) => Some(Ok(chunk)),
            Err(source) => Some(Err(GeminiStreamError::Chunk(source))),
        }
    }

    fn absorb(&mut self, bytes: &[u8]) {
        if let Err(source) = self.lines.push(bytes, &mut self.pending) {
            self.error = Some(GeminiStreamError::Line { source });
        }
    }

    fn end(&mut self) {
        self.lines.flush(&mut self.pending);
        self.ended = true;
    }
}

// LiteLLM's make_call: the caller's handler wins, and raise_for_status translates a 4xx or 5xx first
pub async fn make_call(
    gemini_client: Option<&dyn GeminiPost>,
    client: &dyn GeminiPost,
    call: &StreamCall<'_>,
) -> Result<OpenedReply, GeminiStreamError> {
    let opened = send_stream(gemini_client.unwrap_or(client), call).await?;
    if opened.status < 400 {
        return plain(opened).await;
    }
    let message = translate_exception_str(failure_text(opened.body).await?);
    Err(refused(opened.status, &opened.headers, message))
}

// LiteLLM's make_sync_call: one explicit 200/201 gate and no exception translation
pub async fn make_sync_call(
    gemini_client: Option<&dyn GeminiPost>,
    client: &dyn GeminiPost,
    call: &StreamCall<'_>,
) -> Result<OpenedReply, GeminiStreamError> {
    let opened = send_stream(gemini_client.unwrap_or(client), call).await?;
    match opened.status {
        200 | 201 => Ok(opened),
        status => Err(refused(
            status,
            &opened.headers,
            failure_text(opened.body).await?,
        )),
    }
}

// both calls keep 200 and 201; any other status hands its body back as the provider's message
async fn plain(opened: OpenedReply) -> Result<OpenedReply, GeminiStreamError> {
    if opened.status == 200 || opened.status == 201 {
        return Ok(opened);
    }
    let message = failure_text(opened.body).await?;
    Err(refused(opened.status, &opened.headers, message))
}

fn refused(status: u16, headers: &HeaderMap, message: String) -> GeminiStreamError {
    GeminiStreamError::Http(GeminiHttpError::Provider(get_error_class(
        message, status, headers,
    )))
}

async fn send_stream(
    client: &dyn GeminiPost,
    call: &StreamCall<'_>,
) -> Result<OpenedReply, GeminiStreamError> {
    let opened = client
        .open(PostRequest {
            url: call.url,
            headers: call.headers,
            body: call.body,
            timeout: ClientTimeout::HandlerDefault,
        })
        .await
        .map_err(GeminiHttpError::transport)?;
    tracing::debug!(
        model = call.model,
        status = opened.status,
        "opened the Gemini response stream"
    );
    Ok(opened)
}

// LiteLLM reads the failure body whole; the byte limit keeps a hostile one from growing without end
async fn failure_text(mut chunks: Chunks) -> Result<String, GeminiStreamError> {
    let mut collected = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let bytes = chunk.map_err(GeminiStreamError::body_error)?;
        if collected.len().saturating_add(bytes.len()) > MAX_RESPONSE_BYTES {
            return Err(GeminiStreamError::TooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        collected.extend_from_slice(&bytes);
    }
    Ok(decode_text(&collected))
}
