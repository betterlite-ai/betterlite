use std::collections::VecDeque;

use betterlite_http::error::HttpError;
use betterlite_prompts::arguments::json::Json;
use futures_util::stream::{BoxStream, StreamExt as _};
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
    check_stream_status(head.status, &String::from_utf8_lossy(&head.body))?;
    Ok(OciStream::new(head.chunks.unwrap_or_else(|| {
        futures_util::stream::empty::<Result<Vec<u8>, HttpError>>().boxed()
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
                if matches!(item, StreamItem::Done) {
                    self.done = true;
                }
                return Ok(Some(item));
            }
            if self.done {
                return Ok(None);
            }
            match self.chunks.next().await {
                Some(Ok(bytes)) => self.feed(&bytes).map_err(DispatchError::Oci)?,
                Some(Err(source)) => return Err(DispatchError::Stream { source }),
                None => {
                    self.finish().map_err(DispatchError::Oci)?;
                    if self.pending.is_empty() {
                        return Ok(None);
                    }
                }
            }
        }
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
        if !self.carry.is_empty() {
            let text = String::from_utf8_lossy(&self.carry).into_owned();
            let items = self.decoder.push(&text)?;
            self.pending.extend(items);
            self.carry.clear();
        }
        let items = self.decoder.finish()?;
        self.pending.extend(items);
        self.done = true;
        Ok(())
    }
}

// httpx's iter_text decodes lossily and carries an incomplete UTF-8 sequence over to the next read
fn split_text(bytes: &[u8]) -> (String, usize) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), bytes.len()),
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            (String::from_utf8_lossy(&bytes[..valid]).into_owned(), valid)
        }
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), bytes.len()),
    }
}
