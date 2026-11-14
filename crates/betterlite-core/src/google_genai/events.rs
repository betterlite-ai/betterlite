use std::collections::VecDeque;
use std::mem;

use betterlite_openai::aleph_alpha::lines::LineDecoder;

use super::streaming::GenAiStreamError;

// LiteLLM's _next_google_genai_sse_chunk: a blank line closes the event it follows, and the lines open at EOF form one more
pub(crate) struct GenAiEventDecoder {
    decoder: LineDecoder,
    lines: VecDeque<String>,
    event_lines: Vec<String>,
    input_done: bool,
}

impl GenAiEventDecoder {
    pub(crate) fn new(line_limit: usize) -> Self {
        Self {
            decoder: LineDecoder::new(line_limit),
            lines: VecDeque::new(),
            event_lines: Vec::new(),
            input_done: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<(), GenAiStreamError> {
        self.decoder
            .push(chunk, &mut self.lines)
            .map_err(GenAiStreamError::Lines)
    }

    pub(crate) fn finish(&mut self) {
        self.decoder.flush(&mut self.lines);
        self.input_done = true;
    }

    pub(crate) fn next_event(&mut self) -> Option<Vec<u8>> {
        while let Some(line) = self.lines.pop_front() {
            if line.is_empty() {
                if self.event_lines.is_empty() {
                    continue;
                }
                return Some(encode_event(mem::take(&mut self.event_lines)));
            }
            self.event_lines.push(line);
        }
        if self.input_done && !self.event_lines.is_empty() {
            return Some(encode_event(mem::take(&mut self.event_lines)));
        }
        None
    }
}

fn encode_event(event_lines: Vec<String>) -> Vec<u8> {
    let mut event = event_lines.join("\n");
    event.push_str("\n\n");
    event.into_bytes()
}
