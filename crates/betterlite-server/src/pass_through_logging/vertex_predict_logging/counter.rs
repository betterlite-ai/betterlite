use std::sync::Arc;

use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;

// LiteLLM's token_counter reads every Vertex model name as gpt-3.5-turbo and counts with cl100k_base; the engine is built once per logger.
pub(super) struct Cl100kCounter {
    engine: Engine,
}

impl Cl100kCounter {
    // The bundled asset is embedded and valid; a build that fails leaves the counter out and the embed handler reports it.
    pub(super) fn bundled() -> Option<Self> {
        let vocabulary = assets::bundled(Encoding::Cl100kBase)?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary).ok()?;
        Some(Self {
            engine: Engine::Tiktoken(Arc::new(engine)),
        })
    }
}

impl TextTokenCounter for Cl100kCounter {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        let ids = self.engine.encode(text)?;
        u32::try_from(ids.len()).map_err(|_error| CountError::Overflow)
    }
}
