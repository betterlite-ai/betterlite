use std::collections::VecDeque;
use std::mem;
use std::str;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
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
pub fn decode_lines(response: StreamResponse) -> LineStream {
    unfold(
        LineState {
            body: response.body_stream(),
            pending: VecDeque::new(),
            decoder: Decoder::default(),
            finished: false,
        },
        next_line,
    )
    .boxed()
}

#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    let mut partial = Vec::new();
    let mut text = String::new();
    decode_utf8(&mut partial, bytes, &mut text);
    if !partial.is_empty() {
        text.push('\u{fffd}');
    }
    text
}

struct LineState {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pending: VecDeque<String>,
    decoder: Decoder,
    finished: bool,
}

async fn next_line(mut state: LineState) -> Option<(Result<String, AlephAlphaError>, LineState)> {
    loop {
        if let Some(line) = state.pending.pop_front() {
            return Some((Ok(line), state));
        }
        if state.finished {
            return None;
        }
        match state.body.next().await {
            Some(Ok(chunk)) => state.decoder.push(&chunk, &mut state.pending),
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

#[derive(Debug, Default)]
struct Decoder {
    partial: Vec<u8>,
    buffer: String,
    trailing_cr: bool,
}

impl Decoder {
    fn push(&mut self, chunk: &[u8], lines: &mut VecDeque<String>) {
        let mut text = String::new();
        decode_utf8(&mut self.partial, chunk, &mut text);
        self.split(&text, lines);
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

    fn flush(&mut self, lines: &mut VecDeque<String>) {
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

fn decode_utf8(partial: &mut Vec<u8>, chunk: &[u8], text: &mut String) {
    partial.extend_from_slice(chunk);
    let mut start = 0_usize;
    loop {
        let remaining = partial.get(start..).unwrap_or_default();
        let error = match str::from_utf8(remaining) {
            Ok(decoded) => {
                text.push_str(decoded);
                start = partial.len();
                break;
            }
            Err(error) => error,
        };
        let valid_end = start.saturating_add(error.valid_up_to());
        if let Some(valid) = partial.get(start..valid_end)
            && let Ok(decoded) = str::from_utf8(valid)
        {
            text.push_str(decoded);
        }
        if let Some(length) = error.error_len() {
            text.push('\u{fffd}');
            start = valid_end.saturating_add(length);
        } else {
            start = valid_end;
            break;
        }
    }
    if start >= partial.len() {
        partial.clear();
    } else {
        let tail = partial.split_off(start);
        *partial = tail;
    }
}
