mod helpers;

use helpers::*;

use std::collections::VecDeque;
use std::num::TryFromIntError;

use thiserror::Error;

pub const DEFAULT_MAX_RECURSE_DEPTH: u32 = 100;
pub const MAX_SPLIT_OUTPUT_BYTES: usize = 268_435_456;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SplitError {
    #[error("text splitter output exceeds {limit} bytes")]
    OutputTooLarge { limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChunkConfigError {
    #[error("chunk_size must be greater than zero, got {chunk_size}")]
    NonPositiveChunkSize { chunk_size: i64 },
    #[error("chunk_overlap must not be negative, got {chunk_overlap}")]
    NegativeChunkOverlap { chunk_overlap: i64 },
    #[error("chunk_overlap ({chunk_overlap}) must be less than chunk_size ({chunk_size})")]
    OverlapNotLessThanSize { chunk_size: i64, chunk_overlap: i64 },
    #[error("chunk_size {chunk_size} does not fit in this platform's usize")]
    ChunkSizeOutOfRange {
        chunk_size: i64,
        #[source]
        source: TryFromIntError,
    },
    #[error("chunk_overlap {chunk_overlap} does not fit in this platform's usize")]
    ChunkOverlapOutOfRange {
        chunk_overlap: i64,
        #[source]
        source: TryFromIntError,
    },
}

pub trait TextSplitter {
    fn split_text(&self, text: &str) -> Result<Vec<String>, SplitError>;
}

#[derive(Debug)]
pub struct RecursiveCharacterTextSplitter {
    chunk_size: usize,
    chunk_overlap: usize,
    separators: Vec<String>,
}

impl TextSplitter for RecursiveCharacterTextSplitter {
    fn split_text(&self, text: &str) -> Result<Vec<String>, SplitError> {
        let mut output = Vec::new();
        let mut total_bytes = 0;
        self.split_recursive(text, &self.separators, 0, &mut output, &mut total_bytes)?;
        Ok(output)
    }
}

impl RecursiveCharacterTextSplitter {
    pub fn new(
        requested_chunk_size: i64,
        requested_chunk_overlap: i64,
        separators: Vec<String>,
    ) -> Result<Self, ChunkConfigError> {
        if requested_chunk_size <= 0 {
            return Err(ChunkConfigError::NonPositiveChunkSize {
                chunk_size: requested_chunk_size,
            });
        }
        if requested_chunk_overlap < 0 {
            return Err(ChunkConfigError::NegativeChunkOverlap {
                chunk_overlap: requested_chunk_overlap,
            });
        }
        if requested_chunk_overlap >= requested_chunk_size {
            return Err(ChunkConfigError::OverlapNotLessThanSize {
                chunk_size: requested_chunk_size,
                chunk_overlap: requested_chunk_overlap,
            });
        }
        let chunk_size = usize::try_from(requested_chunk_size).map_err(|source| {
            ChunkConfigError::ChunkSizeOutOfRange {
                chunk_size: requested_chunk_size,
                source,
            }
        })?;
        let chunk_overlap = usize::try_from(requested_chunk_overlap).map_err(|source| {
            ChunkConfigError::ChunkOverlapOutOfRange {
                chunk_overlap: requested_chunk_overlap,
                source,
            }
        })?;
        Ok(Self {
            chunk_size,
            chunk_overlap,
            separators,
        })
    }

    pub(crate) fn split_recursive(
        &self,
        text: &str,
        separators: &[String],
        depth: u32,
        output: &mut Vec<String>,
        total_bytes: &mut usize,
    ) -> Result<(), SplitError> {
        if depth > DEFAULT_MAX_RECURSE_DEPTH {
            for chunk in chunk_by_chars(text, self.chunk_size) {
                push_chunk(output, total_bytes, chunk)?;
            }
            return Ok(());
        }

        let (separator, new_separators) = select_separator(text, separators);
        let splits: Vec<&str> = if separator.is_empty() {
            char_slices(text)
        } else {
            text.split(separator).collect()
        };

        self.assemble_chunks(
            &splits,
            separator,
            new_separators,
            depth,
            output,
            total_bytes,
        )
    }

    fn assemble_chunks(
        &self,
        splits: &[&str],
        separator: &str,
        new_separators: &[String],
        depth: u32,
        output: &mut Vec<String>,
        total_bytes: &mut usize,
    ) -> Result<(), SplitError> {
        let mut good_splits: VecDeque<&str> = VecDeque::new();
        for &split in splits {
            if split.chars().count() < self.chunk_size {
                good_splits.push_back(split);
                continue;
            }
            if !good_splits.is_empty() {
                self.merge_splits(&good_splits, separator, output, total_bytes)?;
                good_splits.clear();
            }
            if new_separators.is_empty() {
                for chunk in self.force_split(split) {
                    push_chunk(output, total_bytes, chunk)?;
                }
            } else {
                let next_depth = depth.saturating_add(1);
                self.split_recursive(split, new_separators, next_depth, output, total_bytes)?;
            }
        }
        if !good_splits.is_empty() {
            self.merge_splits(&good_splits, separator, output, total_bytes)?;
        }
        Ok(())
    }

    fn merge_splits(
        &self,
        splits: &VecDeque<&str>,
        separator: &str,
        output: &mut Vec<String>,
        total_bytes: &mut usize,
    ) -> Result<(), SplitError> {
        let separator_len = separator.chars().count();
        let mut current_chunk: VecDeque<&str> = VecDeque::new();
        let mut current_length: usize = 0;

        for &split in splits {
            let split_len = split.chars().count();
            let sep_len = if current_chunk.is_empty() {
                0
            } else {
                separator_len
            };
            let projected_length = current_length
                .saturating_add(split_len)
                .saturating_add(sep_len);
            if projected_length > self.chunk_size && !current_chunk.is_empty() {
                push_joined(output, total_bytes, &current_chunk, separator)?;
                current_length =
                    self.evict_overlap(&mut current_chunk, current_length, separator_len);
            }
            current_chunk.push_back(split);
            current_length = current_length
                .saturating_add(split_len)
                .saturating_add(sep_len);
        }

        if !current_chunk.is_empty() {
            push_joined(output, total_bytes, &current_chunk, separator)?;
        }
        Ok(())
    }

    fn evict_overlap(
        &self,
        current_chunk: &mut VecDeque<&str>,
        current_length: usize,
        separator_len: usize,
    ) -> usize {
        let mut remaining = current_length;
        while remaining > self.chunk_overlap && current_chunk.len() > 1 {
            let removed = current_chunk.pop_front().unwrap_or_default();
            remaining =
                remaining.saturating_sub(removed.chars().count().saturating_add(separator_len));
        }
        remaining
    }

    fn force_split(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let total = chars.len();
        let mut chunks = Vec::new();
        let mut start = 0_usize;
        let mut done = total == 0;
        while !done {
            let end = start.saturating_add(self.chunk_size);
            let clamped_end = end.min(total);
            let chunk = chars
                .get(start..clamped_end)
                .unwrap_or_default()
                .iter()
                .collect::<String>();
            let trimmed = chunk.trim_matches(is_strip_whitespace);
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_owned());
            }
            if end < total {
                start = end.saturating_sub(self.chunk_overlap);
            } else {
                done = true;
            }
        }
        chunks
    }
}
