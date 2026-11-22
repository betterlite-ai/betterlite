use std::mem;

use betterlite_core::agents::json::AgentJson;

// lines end at LF, CRLF or a lone CR, as the SSE spec says
#[derive(Debug, Default)]
pub struct LineSplitter {
    pub(super) pending: Vec<u8>,
    // a CR that ended the bytes so far; a LF that comes next belongs to the same line break
    trailing_cr: bool,
}

impl LineSplitter {
    #[must_use]
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in chunk {
            let after_cr = mem::take(&mut self.trailing_cr);
            if after_cr {
                lines.push(decode_line(&mem::take(&mut self.pending)));
            }
            match byte {
                b'\n' if after_cr => {}
                b'\r' => self.trailing_cr = true,
                b'\n' => lines.push(decode_line(&mem::take(&mut self.pending))),
                other => self.pending.push(other),
            }
        }
        lines
    }

    #[must_use]
    pub fn finish(self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            Some(decode_line(&self.pending))
        }
    }
}

// httpx decodes with errors="replace", one replacement character per invalid sequence like from_utf8_lossy
fn decode_line(bytes: &[u8]) -> String {
    let mut line = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        line.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            line.push(char::REPLACEMENT_CHARACTER);
        }
    }
    line
}

// Python's str.strip also drops the separators U+001C to U+001F, which Rust does not count as whitespace
#[must_use]
pub fn event_from_line(line: &str) -> Option<AgentJson> {
    let python_space =
        |character: char| character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character);
    let payload = line
        .trim_matches(python_space)
        .strip_prefix("data:")?
        .trim_matches(python_space);
    serde_json::from_str(payload).ok()
}

mod tests;
