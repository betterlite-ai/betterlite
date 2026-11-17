use std::collections::VecDeque;
use std::mem;
use std::ops::Range;

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream, try_unfold};
use http::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};

use crate::vertex_batch_download::policy::{DownloadPolicy, download_policy};
use crate::vertex_batch_download::records::row::transform_line;
use crate::vertex_batch_download::records::{ConversionContext, embeddings_content};
use crate::vertex_batch_rows::decode::{
    decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
};
use crate::vertex_batch_rows::paths::model_from_managed_gcs_url;

// LiteLLM stops looking for a first row once this much prefix holds no completed line
pub const PEEK_LIMIT_BYTES: usize = 0x0200_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    Undecided,
    Passthrough,
    Lines,
    Embeddings,
}

#[derive(Default)]
enum Phase {
    Peek {
        buffered: Vec<u8>,
    },
    Lines {
        pending: Vec<u8>,
        emitted: bool,
    },
    Collect {
        buffered: Vec<u8>,
    },
    Passthrough,
    #[default]
    Finished,
}

pub struct DownloadDecoder {
    mode: DownloadMode,
    phase: Phase,
    headers: HeaderMap,
    model: Option<String>,
    peek_limit: usize,
    context: ConversionContext,
}

impl DownloadDecoder {
    #[must_use]
    pub fn new(
        headers: HeaderMap,
        model: Option<String>,
        peek_limit_bytes: usize,
        context: ConversionContext,
    ) -> Self {
        Self {
            mode: DownloadMode::Undecided,
            phase: Phase::Peek {
                buffered: Vec::new(),
            },
            headers,
            model,
            peek_limit: peek_limit_bytes,
            context,
        }
    }

    #[must_use]
    pub const fn mode(&self) -> DownloadMode {
        self.mode
    }

    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        match mem::take(&mut self.phase) {
            Phase::Peek { mut buffered } => {
                buffered.extend_from_slice(chunk);
                self.decide(Phase::Peek { buffered }, false)
            }
            Phase::Lines {
                mut pending,
                emitted,
            } => {
                pending.extend_from_slice(chunk);
                self.split_lines(Phase::Lines { pending, emitted }, false)
            }
            Phase::Collect { mut buffered } => {
                buffered.extend_from_slice(chunk);
                self.phase = Phase::Collect { buffered };
                Vec::new()
            }
            Phase::Passthrough => {
                self.phase = Phase::Passthrough;
                chunk.to_vec()
            }
            Phase::Finished => Vec::new(),
        }
    }

    fn finish_input(&mut self) -> Vec<u8> {
        match mem::take(&mut self.phase) {
            Phase::Peek { buffered } => self.decide(Phase::Peek { buffered }, true),
            Phase::Lines { pending, emitted } => {
                self.split_lines(Phase::Lines { pending, emitted }, true)
            }
            Phase::Collect { buffered } => self.collect(buffered),
            Phase::Passthrough | Phase::Finished => Vec::new(),
        }
    }

    // the completed-line check runs before the size check, so a line arriving past the limit still counts
    fn decide(&mut self, peeking: Phase, at_eof: bool) -> Vec<u8> {
        let Phase::Peek { buffered } = peeking else {
            return Vec::new();
        };
        let Some(range) = first_row(&buffered, at_eof) else {
            if !at_eof && buffered.len() <= self.peek_limit {
                self.phase = Phase::Peek { buffered };
                return Vec::new();
            }
            return self.keep_original(buffered);
        };
        self.classify(buffered, range)
    }

    // the first row selects the conversion; a row LiteLLM cannot classify keeps every byte as it arrived
    fn classify(&mut self, buffered: Vec<u8>, range: Range<usize>) -> Vec<u8> {
        let Some(row) = buffered
            .get(range)
            .and_then(|line| decode_vertex_batch_row(line).ok())
        else {
            return self.keep_original(buffered);
        };
        if is_embeddings_output_row(&row) {
            self.mode = DownloadMode::Embeddings;
            self.phase = Phase::Collect { buffered };
            return Vec::new();
        }
        if !is_generate_output_row(&row) {
            return self.keep_original(buffered);
        }
        self.mode = DownloadMode::Lines;
        self.headers.remove(CONTENT_LENGTH);
        self.split_lines(
            Phase::Lines {
                pending: buffered,
                emitted: false,
            },
            false,
        )
    }

    fn keep_original(&mut self, buffered: Vec<u8>) -> Vec<u8> {
        self.mode = DownloadMode::Passthrough;
        self.phase = Phase::Passthrough;
        buffered
    }

    // one partial line is held, and each completed line is converted or replayed exactly as it arrived
    fn split_lines(&mut self, lines: Phase, at_eof: bool) -> Vec<u8> {
        let Phase::Lines {
            mut pending,
            mut emitted,
        } = lines
        else {
            return Vec::new();
        };
        let mut output = Vec::new();
        if let Some(last) = pending.iter().rposition(|byte| *byte == b'\n') {
            let tail = pending.split_off(last.saturating_add(1));
            for raw in pending
                .get(..last)
                .unwrap_or_default()
                .split(|byte| *byte == b'\n')
            {
                self.emit_line(raw.trim_ascii(), &mut output, &mut emitted);
            }
            pending = tail;
        }
        if at_eof {
            self.emit_line(pending.trim_ascii(), &mut output, &mut emitted);
            pending = Vec::new();
        }
        self.phase = Phase::Lines { pending, emitted };
        output
    }

    // a row that fails to transform is passed through raw, like the buffered path's whole-file fallback
    fn emit_line(&mut self, line: &[u8], output: &mut Vec<u8>, emitted: &mut bool) {
        if line.is_empty() {
            return;
        }
        if *emitted {
            output.push(b'\n');
        }
        match transform_line(line, &mut self.context) {
            Ok(transformed) => output.extend_from_slice(&transformed),
            Err(_failure) => output.extend_from_slice(line),
        }
        *emitted = true;
    }

    // streamed embeddings read the whole file, so the one chunk carries a recalculated length even
    // when the conversion falls back to the original bytes
    fn collect(&mut self, buffered: Vec<u8>) -> Vec<u8> {
        let transformed = embeddings_content(&buffered, self.model.as_deref(), &mut self.context);
        let content = transformed.unwrap_or(buffered);
        self.headers
            .insert(CONTENT_LENGTH, HeaderValue::from(content.len()));
        content
    }
}

#[must_use]
pub fn decode_vertex_batch_download_chunk(
    mut decoder: DownloadDecoder,
    chunk: &[u8],
) -> (DownloadDecoder, Vec<u8>) {
    let output = decoder.push(chunk);
    (decoder, output)
}

#[must_use]
pub fn finish_vertex_batch_download(mut decoder: DownloadDecoder) -> (DownloadDecoder, Vec<u8>) {
    let output = decoder.finish_input();
    (decoder, output)
}

// LiteLLM re-reads the whole prefix after every chunk, and an empty line never counts as the first
fn first_row(buffered: &[u8], at_eof: bool) -> Option<Range<usize>> {
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

pub struct FileContentStreamingResult {
    pub stream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub headers: HeaderMap,
}

// the first row decides the mode before a byte leaves, and the owned source moves into the result
// stream so dropping it closes the upstream iterator in every phase
pub async fn transform_file_content_stream(
    chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    request_url: &str,
    headers: HeaderMap,
    disabled: bool,
    context: ConversionContext,
) -> Result<FileContentStreamingResult, HttpError> {
    if download_policy(disabled, request_url) == DownloadPolicy::Original {
        return Ok(FileContentStreamingResult {
            stream: chunks,
            headers,
        });
    }
    let decoder = DownloadDecoder::new(
        headers,
        model_from_managed_gcs_url(request_url),
        PEEK_LIMIT_BYTES,
        context,
    );
    let peeked = peek(Peeked {
        chunks,
        decoder,
        pending: VecDeque::new(),
        ended: false,
    })
    .await?;
    if peeked.decoder.mode() == DownloadMode::Embeddings {
        return embeddings_result(peeked).await;
    }
    let Peeked {
        chunks: source,
        decoder,
        pending,
        ended,
    } = peeked;
    let headers = decoder.headers().clone();
    let stream = try_unfold(
        StreamState {
            chunks: source,
            decoder,
            pending,
            ended,
        },
        next_output,
    )
    .boxed();
    Ok(FileContentStreamingResult { stream, headers })
}

struct Peeked {
    chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    decoder: DownloadDecoder,
    pending: VecDeque<Vec<u8>>,
    ended: bool,
}

// reads until the first row decides the mode, keeping every byte read for the replay
async fn peek(mut peeked: Peeked) -> Result<Peeked, HttpError> {
    while peeked.decoder.mode() == DownloadMode::Undecided {
        match peeked.chunks.next().await {
            Some(Ok(chunk)) => {
                let (decoder, output) = decode_vertex_batch_download_chunk(peeked.decoder, &chunk);
                peeked.decoder = decoder;
                if !output.is_empty() {
                    peeked.pending.push_back(output);
                }
            }
            Some(Err(error)) => return Err(error),
            None => {
                let (decoder, output) = finish_vertex_batch_download(peeked.decoder);
                peeked.decoder = decoder;
                if !output.is_empty() {
                    peeked.pending.push_back(output);
                }
                peeked.ended = true;
            }
        }
    }
    Ok(peeked)
}

// streamed embeddings need the whole file, and LiteLLM answers them in one chunk with a new length
async fn embeddings_result(mut peeked: Peeked) -> Result<FileContentStreamingResult, HttpError> {
    if !peeked.ended {
        while let Some(chunk) = peeked.chunks.next().await {
            let chunk = chunk?;
            let (decoder, _output) = decode_vertex_batch_download_chunk(peeked.decoder, &chunk);
            peeked.decoder = decoder;
        }
        let (decoder, output) = finish_vertex_batch_download(peeked.decoder);
        peeked.decoder = decoder;
        if !output.is_empty() {
            peeked.pending.push_back(output);
        }
    }
    let headers = peeked.decoder.headers().clone();
    let single: Vec<Result<Vec<u8>, HttpError>> = peeked.pending.into_iter().map(Ok).collect();
    Ok(FileContentStreamingResult {
        stream: stream::iter(single).boxed(),
        headers,
    })
}

struct StreamState {
    chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    decoder: DownloadDecoder,
    pending: VecDeque<Vec<u8>>,
    ended: bool,
}

async fn next_output(mut state: StreamState) -> Result<Option<(Vec<u8>, StreamState)>, HttpError> {
    loop {
        if let Some(bytes) = state.pending.pop_front() {
            return Ok(Some((bytes, state)));
        }
        if state.ended {
            return Ok(None);
        }
        match state.chunks.next().await {
            Some(Ok(chunk)) => {
                let (decoder, output) = decode_vertex_batch_download_chunk(state.decoder, &chunk);
                state.decoder = decoder;
                if !output.is_empty() {
                    return Ok(Some((output, state)));
                }
            }
            Some(Err(error)) => return Err(error),
            None => {
                let (decoder, output) = finish_vertex_batch_download(state.decoder);
                state.decoder = decoder;
                state.ended = true;
                if !output.is_empty() {
                    return Ok(Some((output, state)));
                }
            }
        }
    }
}

mod tests;
