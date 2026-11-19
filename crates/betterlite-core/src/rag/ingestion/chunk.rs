use std::str;

use betterlite_config::rag::document::chunking::{
    DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE, RAGChunkingStrategy,
};
use serde_json::value::RawValue;
use thiserror::Error;

use crate::rag::document::pdf::extract_text_from_pdf;
use crate::rag::document::splitter::output::SplitError;
use crate::rag::document::splitter::{
    ChunkConfigError, RecursiveCharacterTextSplitter, TextSplitter as _,
};

#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("the chunking configuration is invalid")]
    Configuration(#[from] ChunkConfigError),
    #[error("the text could not be split")]
    Split(#[from] SplitError),
    #[error("the chunking strategy does not match its declared shape")]
    Strategy(#[source] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    pub size: i64,
    pub overlap: i64,
    pub separators: Vec<String>,
}

impl ChunkPlan {
    pub fn from_strategy(strategy: &RawValue) -> Result<Self, ChunkError> {
        let parsed: RAGChunkingStrategy =
            serde_json::from_str(strategy.get()).map_err(ChunkError::Strategy)?;
        Ok(Self {
            size: parsed.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
            overlap: parsed.chunk_overlap.unwrap_or(DEFAULT_CHUNK_OVERLAP),
            separators: parsed.separators,
        })
    }
}

pub fn split(text: &str, plan: ChunkPlan) -> Result<Vec<String>, ChunkError> {
    RecursiveCharacterTextSplitter::new(plan.size, plan.overlap, plan.separators)?
        .split_text(text)
        .map_err(ChunkError::Split)
}

#[must_use]
pub(crate) fn decode_file_content(file_content: &[u8]) -> Option<String> {
    match str::from_utf8(file_content) {
        Ok(text) => Some(text.to_owned()),
        Err(_) if file_content.starts_with(b"%PDF") => extract_text_from_pdf(file_content),
        Err(_) => None,
    }
}
