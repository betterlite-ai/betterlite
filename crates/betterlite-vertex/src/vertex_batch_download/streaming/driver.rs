use std::collections::VecDeque;
use std::ops::Range;

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::decode_download_chunk;
use super::finish_download;
use super::{DownloadDecoder, DownloadMode, FileContentStreamingResult};

pub(super) struct DownloadStream {
    pub(super) chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub(super) decoder: DownloadDecoder,
    pub(super) pending: VecDeque<Vec<u8>>,
    pub(super) ended: bool,
}

impl DownloadStream {
    // one network read feeds the decoder, and the end of input flushes what is left
    async fn read_chunk(&mut self) -> Result<(), HttpError> {
        match self.chunks.next().await {
            Some(Ok(chunk)) => {
                let output = decode_download_chunk(&mut self.decoder, &chunk);
                if !output.is_empty() {
                    self.pending.push_back(output);
                }
                Ok(())
            }
            Some(Err(error)) => Err(error),
            None => {
                let output = finish_download(&mut self.decoder);
                self.ended = true;
                if !output.is_empty() {
                    self.pending.push_back(output);
                }
                Ok(())
            }
        }
    }
}

// reading stops once the first row decides the mode, and every byte read stays queued for the replay
pub(super) async fn peek(mut stream: DownloadStream) -> Result<DownloadStream, HttpError> {
    while stream.decoder.mode == DownloadMode::Undecided && !stream.ended {
        stream.read_chunk().await?;
    }
    Ok(stream)
}

// streamed embeddings need the whole file, and LiteLLM answers them in one chunk with a new length
pub(super) async fn embeddings_result(
    mut stream: DownloadStream,
) -> Result<FileContentStreamingResult, HttpError> {
    while !stream.ended {
        stream.read_chunk().await?;
    }
    let headers = stream.decoder.headers.clone();
    let single: Vec<Result<Vec<u8>, HttpError>> = stream.pending.into_iter().map(Ok).collect();
    Ok(FileContentStreamingResult {
        stream: stream::iter(single).boxed(),
        headers,
    })
}

// every produced byte leaves before the next read, and nothing is read past the end
pub(super) async fn next_output(
    mut stream: DownloadStream,
) -> Result<Option<(Vec<u8>, DownloadStream)>, HttpError> {
    loop {
        if let Some(bytes) = stream.pending.pop_front() {
            return Ok(Some((bytes, stream)));
        }
        if stream.ended {
            return Ok(None);
        }
        stream.read_chunk().await?;
    }
}

// LiteLLM re-reads the whole prefix after every chunk, and an empty line never counts as the first
pub(super) fn first_row(buffered: &[u8], at_eof: bool) -> Option<Range<usize>> {
    let mut start = 0;
    loop {
        let rest = buffered.get(start..).unwrap_or_default();
        let Some(offset) = rest.iter().position(|byte| *byte == b'\n') else {
            let tail = rest.trim_ascii();
            return (at_eof && !tail.is_empty()).then_some(start..buffered.len());
        };
        let end = start.saturating_add(offset);
        if !buffered
            .get(start..end)
            .unwrap_or_default()
            .trim_ascii()
            .is_empty()
        {
            return Some(start..end);
        }
        start = end.saturating_add(1);
    }
}
