use std::collections::VecDeque;
use std::str::from_utf8;

use betterlite_http::error::HttpError;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::gitlab_client::payload::decode_text;
use futures_util::stream::{BoxStream, StreamExt as _, empty};
use http::{HeaderMap, Method};

use crate::oci_chat::stream::{STREAMING_TIMEOUT, SseDecoder, StreamItem, check_stream_status};
use crate::oci_common::OciError;

use super::error::DispatchError;
use super::transport::{PostTransport, WireRequest};

pub struct OciStreamRequest<'request> {
    pub api_base: &'request str,
    pub headers: HeaderMap,
    pub data: &'request Json,
    // signing produced the body already; a call without a signer sends the JSON of the request
    pub signed_body: Option<Vec<u8>>,
}

// LiteLLM's get_sync_custom_stream_wrapper: POST the signed or JSON body and hand the text stream to the SSE decoder
pub async fn open_stream(
    request: OciStreamRequest<'_>,
    transport: &dyn PostTransport,
) -> Result<OciStream, DispatchError> {
    let body = match request.signed_body {
        Some(signed) => signed,
        None => {
            serde_json::to_vec(request.data).map_err(|source| DispatchError::Encode { source })?
        }
    };
    let head = transport
        .post_stream(&WireRequest {
            method: Method::POST,
            url: request.api_base,
            headers: &request.headers,
            body: &body,
            deadline: STREAMING_TIMEOUT,
        })
        .await?;
    check_stream_status(head.status, &decode_text(&head.body))?;
    Ok(OciStream::new(head.chunks.unwrap_or_else(|| {
        empty::<Result<Vec<u8>, HttpError>>().boxed()
    })))
}

pub struct OciStream {
    chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    decoder: SseDecoder,
    pending: VecDeque<StreamItem>,
    carry: Vec<u8>,
    done: bool,
}

impl OciStream {
    fn new(chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>) -> Self {
        Self {
            chunks,
            decoder: SseDecoder::default(),
            pending: VecDeque::new(),
            carry: Vec::new(),
            done: false,
        }
    }

    pub async fn next(&mut self) -> Result<Option<StreamItem>, DispatchError> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                self.done |= matches!(item, StreamItem::Done);
                return Ok(Some(item));
            }
            if self.done {
                return Ok(None);
            }
            let Some(chunk) = self.chunks.next().await else {
                return self.finish_tail();
            };
            match chunk {
                Ok(bytes) => self.feed(&bytes).map_err(DispatchError::Oci)?,
                Err(source) => return Err(DispatchError::Stream { source }),
            }
        }
    }

    // the decoder tail can still hold items, so the stream ends only when none is left
    fn finish_tail(&mut self) -> Result<Option<StreamItem>, DispatchError> {
        self.finish().map_err(DispatchError::Oci)?;
        if self.pending.is_empty() {
            return Ok(None);
        }
        Ok(self.pending.pop_front())
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), OciError> {
        self.carry.extend_from_slice(bytes);
        let (text, consumed) = split_text(&self.carry);
        let items = self.decoder.push(&text)?;
        self.pending.extend(items);
        if consumed == self.carry.len() {
            self.carry.clear();
        } else {
            self.carry.drain(..consumed);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), OciError> {
        // the lossy text goes in even when no bytes are carried, since an empty push leaves the decoder as it was
        let tail = decode_text(&self.carry);
        self.carry.clear();
        let items = self.decoder.push(&tail)?;
        self.pending.extend(items);
        let finished = self.decoder.finish()?;
        self.pending.extend(finished);
        self.done = true;
        Ok(())
    }
}

// httpx's iter_text decodes lossily and carries an incomplete UTF-8 sequence over to the next read
fn split_text(bytes: &[u8]) -> (String, usize) {
    let error = match from_utf8(bytes) {
        Ok(text) => return (text.to_owned(), bytes.len()),
        Err(error) => error,
    };
    let valid = error.valid_up_to();
    match error.error_len() {
        None => (decode_text(bytes.get(..valid).unwrap_or_default()), valid),
        Some(_) => (decode_text(bytes), bytes.len()),
    }
}
