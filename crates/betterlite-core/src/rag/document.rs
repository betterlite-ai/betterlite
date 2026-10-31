use std::panic::{AssertUnwindSafe, catch_unwind};

use betterlite_config::rag::document::{
    INGESTION_PROVIDERS, IngestionProvider, ProviderIdentity as _,
};
use thiserror::Error;

pub const DEFAULT_CHUNK_SIZE: usize = 1000;
pub const DEFAULT_CHUNK_OVERLAP: usize = 200;
pub const DEFAULT_MAX_RECURSE_DEPTH: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChunkConfigError {
    #[error("chunk_size must be greater than zero, got {chunk_size}")]
    NonPositiveChunkSize { chunk_size: i64 },
    #[error("chunk_overlap must not be negative, got {chunk_overlap}")]
    NegativeChunkOverlap { chunk_overlap: i64 },
    #[error("chunk_overlap ({chunk_overlap}) must be less than chunk_size ({chunk_size})")]
    OverlapNotLessThanSize { chunk_size: i64, chunk_overlap: i64 },
}

pub trait TextSplitter {
    fn split_text(&self, text: &str) -> Vec<String>;
}

#[derive(Debug)]
pub struct RecursiveCharacterTextSplitter {
    chunk_size: usize,
    chunk_overlap: usize,
    separators: Vec<String>,
}

impl TextSplitter for RecursiveCharacterTextSplitter {
    fn split_text(&self, text: &str) -> Vec<String> {
        self.split_recursive(text, &self.separators, 0)
    }
}

impl RecursiveCharacterTextSplitter {
    pub fn new(
        requested_chunk_size: i64,
        requested_chunk_overlap: i64,
        requested_separators: Option<Vec<String>>,
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
        let chunk_size = usize::try_from(requested_chunk_size).unwrap_or(usize::MAX);
        let chunk_overlap = usize::try_from(requested_chunk_overlap).unwrap_or(usize::MAX);
        let separators = match requested_separators {
            Some(given) if !given.is_empty() => given,
            _ => ["\n\n", "\n", " ", ""]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        Ok(Self {
            chunk_size,
            chunk_overlap,
            separators,
        })
    }

    fn split_recursive(&self, text: &str, separators: &[String], depth: u32) -> Vec<String> {
        if depth > DEFAULT_MAX_RECURSE_DEPTH {
            return chunk_by_chars(text, self.chunk_size);
        }

        let (separator, new_separators) = select_separator(text, separators);
        let splits: Vec<&str> = if separator.is_empty() {
            char_slices(text)
        } else {
            text.split(separator).collect()
        };

        self.assemble_chunks(&splits, separator, new_separators, depth)
    }

    fn assemble_chunks(
        &self,
        splits: &[&str],
        separator: &str,
        new_separators: &[String],
        depth: u32,
    ) -> Vec<String> {
        let mut final_chunks: Vec<String> = Vec::new();
        let mut good_splits: Vec<&str> = Vec::new();
        for &split in splits {
            if split.chars().count() < self.chunk_size {
                good_splits.push(split);
                continue;
            }
            if !good_splits.is_empty() {
                final_chunks.extend(self.merge_splits(&good_splits, separator));
                good_splits.clear();
            }
            if new_separators.is_empty() {
                final_chunks.extend(self.force_split(split));
            } else {
                let next_depth = depth.saturating_add(1);
                final_chunks.extend(self.split_recursive(split, new_separators, next_depth));
            }
        }
        if !good_splits.is_empty() {
            final_chunks.extend(self.merge_splits(&good_splits, separator));
        }
        final_chunks
    }

    fn merge_splits(&self, splits: &[&str], separator: &str) -> Vec<String> {
        let separator_len = separator.chars().count();
        let mut chunks: Vec<String> = Vec::new();
        let mut current_chunk: Vec<&str> = Vec::new();
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
                push_joined(&mut chunks, &current_chunk, separator);
                current_length =
                    self.evict_overlap(&mut current_chunk, current_length, separator_len);
            }
            current_chunk.push(split);
            current_length = current_length
                .saturating_add(split_len)
                .saturating_add(sep_len);
        }

        if !current_chunk.is_empty() {
            push_joined(&mut chunks, &current_chunk, separator);
        }
        chunks
    }

    fn evict_overlap(
        &self,
        current_chunk: &mut Vec<&str>,
        current_length: usize,
        separator_len: usize,
    ) -> usize {
        let mut remaining = current_length;
        while remaining > self.chunk_overlap && current_chunk.len() > 1 {
            let removed = current_chunk.remove(0);
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
            let chunk: String = chars
                .get(start..clamped_end)
                .unwrap_or_default()
                .iter()
                .collect::<String>()
                .trim()
                .to_owned();
            if !chunk.is_empty() {
                chunks.push(chunk);
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

fn select_separator<'sep>(text: &str, separators: &'sep [String]) -> (&'sep str, &'sep [String]) {
    for (index, candidate) in separators.iter().enumerate() {
        if candidate.is_empty() {
            return (candidate, &[]);
        }
        if text.contains(candidate.as_str()) {
            let rest_start = index.saturating_add(1);
            return (candidate, separators.get(rest_start..).unwrap_or(&[]));
        }
    }
    (separators.last().map_or("", String::as_str), &[])
}

fn push_joined(chunks: &mut Vec<String>, current_chunk: &[&str], separator: &str) {
    let chunk_text = current_chunk.join(separator).trim().to_owned();
    if !chunk_text.is_empty() {
        chunks.push(chunk_text);
    }
}

fn char_slices(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    for (start, ch) in text.char_indices() {
        let end = start.saturating_add(ch.len_utf8());
        if let Some(slice) = text.get(start..end) {
            result.push(slice);
        }
    }
    result
}

fn chunk_by_chars(text: &str, chunk_size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut index = 0_usize;
    while index < chars.len() {
        let end = index.saturating_add(chunk_size).min(chars.len());
        result.push(chars.get(index..end).unwrap_or_default().iter().collect());
        index = index.saturating_add(chunk_size);
    }
    result
}

#[must_use]
pub fn extract_text_from_pdf(file_content: &[u8]) -> Option<String> {
    let extracted = catch_unwind(AssertUnwindSafe(|| {
        let document = lopdf::Document::load_mem(file_content).ok()?;
        let page_numbers: Vec<u32> = document.get_pages().keys().copied().collect();
        let mut text_parts: Vec<String> = Vec::new();
        for page_number in page_numbers {
            if let Ok(text) = document.extract_text(&[page_number])
                && !text.is_empty()
            {
                text_parts.push(text);
            }
        }
        if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n\n"))
        }
    }));
    extracted.unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IngestionProviderError {
    #[error(
        "Provider '{provider}' is not supported for RAG ingestion. Supported providers: {supported}"
    )]
    Unsupported { provider: String, supported: String },
}

pub fn get_ingestion_class(provider: &str) -> Result<IngestionProvider, IngestionProviderError> {
    if let Some(resolved) = IngestionProvider::from_wire_name(provider) {
        return Ok(resolved);
    }
    let supported = INGESTION_PROVIDERS
        .iter()
        .map(|known| known.wire_name())
        .collect::<Vec<_>>()
        .join(", ");
    Err(IngestionProviderError::Unsupported {
        provider: provider.to_owned(),
        supported,
    })
}

#[path = "document/tests.rs"]
mod tests;
