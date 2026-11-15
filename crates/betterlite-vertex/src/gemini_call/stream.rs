use std::collections::VecDeque;
use std::mem;

use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::lines::LineDecoder;
use futures_util::StreamExt as _;

use crate::gemini_http::error::GeminiStreamError;
use crate::gemini_http::post::{Chunks, OpenedReply};
use crate::gemini_http::stream::{StreamCall, StreamSetup, make_sync_call};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::gemini_stream::chunk::StreamChunk;
use crate::gemini_stream::{GeminiStreamDecoder, StreamOptions};

// LiteLLM's sync CustomStreamWrapper around make_sync_call: nothing is posted until the first chunk is consumed
pub struct SyncGeminiStream {
    state: StreamState,
    options: StreamOptions,
}

enum StreamState {
    Pending(Box<StreamSetup>),
    Reading(Box<Reading>),
    Finished,
}

impl SyncGeminiStream {
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
        let opened = make_sync_call(setup.supplied.as_deref(), setup.pooled.as_ref(), &call).await;
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
                    Err(source) => return Some(self.stop(GeminiStreamError::Chunk(source))),
                }
            }
            if let Some(error) = self.error.take() {
                return Some(self.stop(error));
            }
            // the decoder's last chunk comes out once the body ended, and the stream stays finished after it
            if self.ended {
                self.done = true;
                return match self.decoder.finish() {
                    Ok(None) => None,
                    Ok(Some(chunk)) => Some(Ok(chunk)),
                    Err(source) => Some(Err(GeminiStreamError::Chunk(source))),
                };
            }
            match self.chunks.next().await {
                Some(Ok(bytes)) => self.absorb(&bytes),
                Some(Err(source)) => self.error = Some(body_error(source)),
                None => {
                    self.lines.flush(&mut self.pending);
                    self.ended = true;
                }
            }
        }
    }

    // a failure ends the stream once the caller has seen it
    const fn stop(&mut self, error: GeminiStreamError) -> Result<StreamChunk, GeminiStreamError> {
        self.done = true;
        Err(error)
    }

    fn absorb(&mut self, bytes: &[u8]) {
        if let Err(source) = self.lines.push(bytes, &mut self.pending) {
            self.error = Some(GeminiStreamError::Line { source });
        }
    }
}

// B1316's body classification, mirrored because its helper is private to the HTTP module
fn body_error(source: HttpError) -> GeminiStreamError {
    let timed_out = match &source {
        HttpError::Timeout => true,
        HttpError::Request(error) => error.is_timeout(),
        HttpError::Required { .. }
        | HttpError::DotSegment { .. }
        | HttpError::UnsupportedProxyScheme { .. }
        | HttpError::InvalidProxyUrl { .. }
        | HttpError::Client(_)
        | HttpError::TooLarge { .. } => false,
    };
    if timed_out {
        return GeminiStreamError::Timeout;
    }
    GeminiStreamError::Body { source }
}
