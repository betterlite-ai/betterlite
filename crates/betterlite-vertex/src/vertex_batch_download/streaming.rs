use std::collections::VecDeque;
use std::mem;
use std::ops::Range;

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, try_unfold};
use http::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};

use crate::vertex_batch_download::policy::{DownloadPolicy, download_policy};
use crate::vertex_batch_download::records::row::transform_line;
use crate::vertex_batch_download::records::{ConversionContext, embeddings_content};

use crate::vertex_batch_rows::decode::{
    decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
};
use crate::vertex_batch_rows::paths::model_from_managed_gcs_url;
use driver::{DownloadStream, embeddings_result, first_row, next_output, peek};

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
    pub(crate) mode: DownloadMode,
    phase: Phase,
    pub(crate) headers: HeaderMap,
    model: Option<String>,
    peek_limit: usize,
    context: ConversionContext,
}

impl DownloadDecoder {
    #[must_use]
    pub const fn new(
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
        self.classify(buffered, range, at_eof)
    }

    // the first row selects the conversion; a row LiteLLM cannot classify keeps every byte as it arrived
    fn classify(&mut self, buffered: Vec<u8>, range: Range<usize>, at_eof: bool) -> Vec<u8> {
        let Some(row) = buffered
            .get(range)
            .and_then(|line| decode_vertex_batch_row(line.trim_ascii()).ok())
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
            at_eof,
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

    // streamed embeddings read the whole file, so the one chunk carries a recalculated length on fallback too
    fn collect(&mut self, buffered: Vec<u8>) -> Vec<u8> {
        let transformed = embeddings_content(&buffered, self.model.as_deref(), &mut self.context);
        let content = transformed.unwrap_or(buffered);
        self.headers
            .insert(CONTENT_LENGTH, HeaderValue::from(content.len()));
        content
    }

    // network bytes feed the phase the last chunk left behind, and no chunk at all means end of input
    fn advance(&mut self, chunk: Option<&[u8]>) -> Vec<u8> {
        match mem::take(&mut self.phase) {
            Phase::Peek { mut buffered } => {
                if let Some(bytes) = chunk {
                    buffered.extend_from_slice(bytes);
                }
                self.decide(Phase::Peek { buffered }, chunk.is_none())
            }
            Phase::Lines {
                mut pending,
                emitted,
            } => {
                if let Some(bytes) = chunk {
                    pending.extend_from_slice(bytes);
                }
                self.split_lines(Phase::Lines { pending, emitted }, chunk.is_none())
            }
            Phase::Collect { mut buffered } => match chunk {
                Some(bytes) => {
                    buffered.extend_from_slice(bytes);
                    self.phase = Phase::Collect { buffered };
                    Vec::new()
                }
                None => self.collect(buffered),
            },
            Phase::Passthrough => {
                self.phase = Phase::Passthrough;
                chunk.map_or_else(Vec::new, <[u8]>::to_vec)
            }
            Phase::Finished => Vec::new(),
        }
    }
}

#[must_use]
pub fn decode_download_chunk(decoder: &mut DownloadDecoder, chunk: &[u8]) -> Vec<u8> {
    if chunk.is_empty() {
        return Vec::new();
    }
    decoder.advance(Some(chunk))
}

// the tail line can turn the file into an embeddings one, which then needs the bytes held in the phase
#[must_use]
pub fn finish_download(decoder: &mut DownloadDecoder) -> Vec<u8> {
    let output = decoder.advance(None);
    match mem::replace(&mut decoder.phase, Phase::Finished) {
        Phase::Collect { buffered } => decoder.collect(buffered),
        Phase::Peek { .. } | Phase::Lines { .. } | Phase::Passthrough | Phase::Finished => output,
    }
}

pub struct FileContentStreamingResult {
    pub stream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub headers: HeaderMap,
}

// the first row decides the mode before a byte leaves, and dropping the result stream closes the owned source
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
    let stream = peek(DownloadStream {
        chunks,
        decoder,
        pending: VecDeque::new(),
        ended: false,
    })
    .await?;
    if stream.decoder.mode == DownloadMode::Embeddings {
        return embeddings_result(stream).await;
    }
    let response_headers = stream.decoder.headers.clone();
    let output = try_unfold(stream, next_output).boxed();
    Ok(FileContentStreamingResult {
        stream: output,
        headers: response_headers,
    })
}

mod driver;
mod tests;
