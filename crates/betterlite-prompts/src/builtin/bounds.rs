use super::{Message, PromptError};
use std::io::{self, Write};

const INPUT_BYTES: usize = 0x0001_0000;
pub(super) const OUTPUT_BYTES: usize = 0x0010_0000;

struct Counter {
    remaining: usize,
}
impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(buf.len())
            .ok_or_else(|| io::Error::other("prompt byte limit"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn check(messages: &[Message]) -> Result<(), PromptError> {
    let counter = Counter {
        remaining: INPUT_BYTES,
    };
    serde_json::to_writer(counter, messages).map_err(PromptError::InputLimit)
}
pub(super) fn finish(output: String) -> Result<String, PromptError> {
    if output.len() > OUTPUT_BYTES {
        Err(PromptError::Limit)
    } else {
        Ok(output)
    }
}

mod tests;
