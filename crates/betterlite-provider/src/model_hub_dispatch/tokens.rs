use std::sync::Arc;

use betterlite_openai::huggingface_embedding::response::OrdinaryEncoder;
use betterlite_openai::huggingface_rerank::TextTokenCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

use super::HubDispatchError;

// main.py's _get_encoding and embedding/handler.py L205 both count with tiktoken's cl100k_base, which B195 bundles
pub struct Cl100k {
    engine: Engine,
}

impl Cl100k {
    pub fn new() -> Result<Self, HubDispatchError> {
        let vocabulary =
            assets::bundled(Encoding::Cl100kBase).ok_or(HubDispatchError::MissingVocabulary)?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary)
            .map_err(|source| HubDispatchError::Encoding { source })?;
        Ok(Self {
            engine: Engine::Tiktoken(Arc::new(engine)),
        })
    }
}

impl OrdinaryEncoder for Cl100k {
    fn encode_ordinary(&self, text: &str) -> usize {
        self.engine.encode(text).map_or(0, |ids| ids.len())
    }
}

impl TextTokenCounter for Cl100k {
    fn count(&self, _model: &str, text: &str) -> Result<u64, CountError> {
        let ids = self.engine.encode(text)?;
        u64::try_from(ids.len()).map_err(|_error| CountError::Overflow)
    }
}
