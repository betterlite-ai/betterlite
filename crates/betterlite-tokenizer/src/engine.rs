pub mod assets;
pub mod error;
pub mod fetch;
pub mod hub;
pub mod huggingface;
pub mod limits;
pub mod selection;
pub mod tiktoken;

mod tests;

use std::sync::Arc;

use error::EngineError;
use huggingface::HuggingFaceEngine;
use tiktoken::TiktokenEngine;

#[derive(Debug, Clone)]
pub enum Engine {
    Tiktoken(Arc<TiktokenEngine>),
    HuggingFace(Arc<HuggingFaceEngine>),
}

impl Engine {
    pub fn encode(&self, text: &str) -> Result<Vec<u32>, EngineError> {
        match self {
            Self::Tiktoken(engine) => engine.encode(text),
            Self::HuggingFace(engine) => engine.encode(text),
        }
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String, EngineError> {
        match self {
            Self::Tiktoken(engine) => engine.decode(ids),
            Self::HuggingFace(engine) => engine.decode(ids),
        }
    }
}
