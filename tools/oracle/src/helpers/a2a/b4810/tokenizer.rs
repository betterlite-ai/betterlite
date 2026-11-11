use async_trait::async_trait;
use betterlite_a2a::streaming_iterator::TokenCounter;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

use super::helper_error;
use crate::compare::helper::HelperError;

pub(super) struct Cl100kCounter {
    engine: TiktokenEngine,
}

impl Cl100kCounter {
    pub(super) fn new() -> Result<Self, Box<HelperError>> {
        let data = assets::bundled(Encoding::Cl100kBase).ok_or_else(|| {
            helper_error("TokenizerError", "the cl100k vocabulary is not bundled")
        })?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data)
            .map_err(|error| helper_error("TokenizerError", error.to_string()))?;
        Ok(Self { engine })
    }
}

#[async_trait]
impl TokenCounter for Cl100kCounter {
    async fn count_tokens(&self, text: &str) -> usize {
        self.engine.encode(text).map_or(0, |ids| ids.len())
    }
}
