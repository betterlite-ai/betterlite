use std::collections::VecDeque;
use std::mem;

use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::aleph_alpha::lines::LineDecoder;

use super::streaming::GenAiStreamError;

// LiteLLM's _next_google_genai_sse_chunk: a blank line closes the event it follows, and the lines open at EOF form one more
pub(crate) struct GenAiEventDecoder {
    decoder: LineDecoder,
    lines: VecDeque<String>,
    event_lines: Vec<String>,
    open_bytes: usize,
    line_limit: usize,
    input_done: bool,
}

impl GenAiEventDecoder {
    pub(crate) fn new(line_limit: usize) -> Self {
        Self {
            decoder: LineDecoder::new(line_limit),
            lines: VecDeque::new(),
            event_lines: Vec::new(),
            open_bytes: 0,
            line_limit,
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

    pub(crate) fn next_event(&mut self) -> Result<Option<Vec<u8>>, GenAiStreamError> {
        while let Some(line) = self.lines.pop_front() {
            if line.len() > self.line_limit {
                return self.fail(GenAiStreamError::Lines(AlephAlphaError::LineTooLong {
                    limit: self.line_limit,
                }));
            }
            if line.is_empty() {
                if self.event_lines.is_empty() {
                    continue;
                }
                self.open_bytes = 0;
                return Ok(Some(encode_event(mem::take(&mut self.event_lines))));
            }
            self.open_bytes += line.len() + 1;
            if self.open_bytes + 1 > self.line_limit {
                return self.fail(GenAiStreamError::EventTooLong {
                    limit: self.line_limit,
                });
            }
            self.event_lines.push(line);
        }
        if self.input_done && !self.event_lines.is_empty() {
            self.open_bytes = 0;
            return Ok(Some(encode_event(mem::take(&mut self.event_lines))));
        }
        Ok(None)
    }

    // a stream that failed stops here: whatever it still held is dropped, so no event follows the error
    fn fail(&mut self, error: GenAiStreamError) -> Result<Option<Vec<u8>>, GenAiStreamError> {
        self.lines.clear();
        self.event_lines.clear();
        self.open_bytes = 0;
        self.input_done = true;
        Err(error)
    }
}

fn encode_event(event_lines: Vec<String>) -> Vec<u8> {
    let mut event = event_lines.join("\n");
    event.push_str("\n\n");
    event.into_bytes()
}
