use std::collections::VecDeque;
use std::mem;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use encoding_rs::{CoderResult, Decoder as Utf8Decoder, UTF_8};
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, unfold};

use crate::aleph_alpha::error::AlephAlphaError;

pub type LineStream = BoxStream<'static, Result<String, AlephAlphaError>>;

// the characters Python's str.splitlines breaks on, which httpx's LineDecoder follows
const LINE_BREAKS: [char; 10] = [
    '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

// LiteLLM hands the raw response lines on with iter_lines, which splits the incrementally decoded text
#[must_use]
pub fn decode_lines(response: StreamResponse, line_limit: usize) -> LineStream {
    unfold(
        LineState {
            body: response.body_stream(),
            pending: VecDeque::new(),
            decoder: LineDecoder::new(line_limit),
            oversized: None,
            finished: false,
        },
        next_line,
    )
    .boxed()
}

#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    let mut text = String::new();
    decode_chunk(
        &mut UTF_8.new_decoder_without_bom_handling(),
        bytes,
        true,
        &mut text,
    );
    text
}

fn decode_chunk(decoder: &mut Utf8Decoder, chunk: &[u8], last: bool, text: &mut String) {
    let mut offset = 0_usize;
    loop {
        text.reserve(
            decoder
                .max_utf8_buffer_length(chunk.len().saturating_sub(offset))
                .unwrap_or_default(),
        );
        let (result, read, _replaced) =
            decoder.decode_to_string(chunk.get(offset..).unwrap_or_default(), text, last);
        offset = offset.saturating_add(read);
        if result == CoderResult::InputEmpty {
            return;
        }
    }
}

struct LineState {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pending: VecDeque<String>,
    decoder: LineDecoder,
    oversized: Option<AlephAlphaError>,
    finished: bool,
}

async fn next_line(mut state: LineState) -> Option<(Result<String, AlephAlphaError>, LineState)> {
    loop {
        if let Some(line) = state.pending.pop_front() {
            return Some((Ok(line), state));
        }
        if let Some(failure) = state.oversized.take() {
            state.finished = true;
            return Some((Err(failure), state));
        }
        if state.finished {
            return None;
        }
        match state.body.next().await {
            Some(Ok(chunk)) => {
                // the lines the chunk completed come out before the error for the line it left open
                state.oversized = state.decoder.push(&chunk, &mut state.pending).err();
            }
            Some(Err(error)) => {
                state.finished = true;
                let failure = AlephAlphaError::Transport {
                    source: TransportError::Rejected(error),
                };
                return Some((Err(failure), state));
            }
            None => {
                state.finished = true;
                state.decoder.flush(&mut state.pending);
            }
        }
    }
}

#[derive(Debug)]
pub struct LineDecoder {
    utf8: Utf8Decoder,
    buffer: String,
    trailing_cr: bool,
    line_limit: usize,
}

impl LineDecoder {
    #[must_use]
    pub fn new(line_limit: usize) -> Self {
        Self {
            utf8: UTF_8.new_decoder_without_bom_handling(),
            buffer: String::new(),
            trailing_cr: false,
            line_limit,
        }
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        lines: &mut VecDeque<String>,
    ) -> Result<(), AlephAlphaError> {
        let mut text = String::new();
        decode_chunk(&mut self.utf8, chunk, false, &mut text);
        self.split(&text, lines);
        if self.buffer.len() > self.line_limit {
            return Err(AlephAlphaError::LineTooLong {
                limit: self.line_limit,
            });
        }
        Ok(())
    }

    fn split(&mut self, text: &str, lines: &mut VecDeque<String>) {
        let mut owned = text.to_owned();
        if self.trailing_cr {
            owned.insert(0, '\r');
            self.trailing_cr = false;
        }
        if owned.ends_with('\r') {
            self.trailing_cr = true;
            owned.truncate(owned.len().saturating_sub(1));
        }
        if owned.is_empty() {
            return;
        }
        let trailing_newline = owned
            .chars()
            .next_back()
            .is_some_and(|character| LINE_BREAKS.contains(&character));
        let mut parts = split_lines(&owned);
        if parts.len() == 1 && !trailing_newline {
            if let Some(only) = parts.first() {
                self.buffer.push_str(only);
            }
            return;
        }
        if !self.buffer.is_empty()
            && let Some(first) = parts.first_mut()
        {
            let mut joined = mem::take(&mut self.buffer);
            joined.push_str(first);
            *first = joined;
        }
        if !trailing_newline && let Some(last) = parts.pop() {
            self.buffer = last;
        }
        lines.extend(parts);
    }

    pub fn flush(&mut self, lines: &mut VecDeque<String>) {
        let mut tail = String::new();
        decode_chunk(&mut self.utf8, &[], true, &mut tail);
        self.split(&tail, lines);
        if self.buffer.is_empty() && !self.trailing_cr {
            return;
        }
        let remainder = mem::take(&mut self.buffer);
        self.trailing_cr = false;
        lines.push_back(remainder);
    }
}

fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !LINE_BREAKS.contains(&character) {
            continue;
        }
        lines.push(text.get(start..index).unwrap_or_default().to_owned());
        let mut next = index.saturating_add(character.len_utf8());
        if character == '\r' && matches!(characters.peek(), Some((_, '\n'))) {
            let line_feed = characters.next();
            if line_feed.is_some() {
                next = next.saturating_add(1);
            }
        }
        start = next;
    }
    if start < text.len() {
        lines.push(text.get(start..).unwrap_or_default().to_owned());
    }
    lines
}
