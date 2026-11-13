mod tests;

use std::collections::VecDeque;

use thiserror::Error;

use super::is_strip_whitespace;

pub const MAX_SPLIT_OUTPUT_BYTES: usize = 0x1000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SplitError {
    #[error("text splitter output exceeds {limit} bytes")]
    OutputTooLarge { limit: usize },
}

#[derive(Default)]
pub(crate) struct SplitOutput {
    pub(crate) chunks: Vec<String>,
    total_bytes: usize,
}

impl SplitOutput {
    pub(super) fn push(&mut self, chunk: String) -> Result<(), SplitError> {
        let next = self
            .total_bytes
            .checked_add(chunk.len())
            .ok_or(SplitError::OutputTooLarge {
                limit: MAX_SPLIT_OUTPUT_BYTES,
            })?;
        if next > MAX_SPLIT_OUTPUT_BYTES {
            return Err(SplitError::OutputTooLarge {
                limit: MAX_SPLIT_OUTPUT_BYTES,
            });
        }
        self.total_bytes = next;
        self.chunks.push(chunk);
        Ok(())
    }
}

pub(super) fn push_joined(
    output: &mut SplitOutput,
    current_chunk: &VecDeque<&str>,
    separator: &str,
) -> Result<(), SplitError> {
    let joined = current_chunk
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .join(separator);
    let trimmed = joined.trim_matches(is_strip_whitespace);
    if !trimmed.is_empty() {
        output.push(trimmed.to_owned())?;
    }
    Ok(())
}
