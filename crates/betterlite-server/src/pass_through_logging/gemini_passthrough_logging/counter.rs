use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message as CountMessage;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

// LiteLLM's token_counter normalises these passthrough model names to gpt-3.5-turbo, so every count uses cl100k_base.
#[derive(Clone)]
pub(crate) struct CollectedCounter {
    engine: Engine,
}

impl CollectedCounter {
    pub(crate) fn bundled() -> Option<Self> {
        let vocabulary = assets::bundled(Encoding::Cl100kBase)?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary).ok()?;
        Some(Self {
            engine: Engine::Tiktoken(Arc::new(engine)),
        })
    }

    fn encode_len(&self, text: &str) -> Result<i64, CountError> {
        let ids = self.engine.encode(text)?;
        i64::try_from(ids.len()).map_err(|_conversion| CountError::Overflow)
    }
}

#[async_trait]
impl UsageTokenCounter for CollectedCounter {
    async fn count_response_text(&self, _model: &str, text: &str) -> Result<i64, CountError> {
        self.encode_len(text)
    }

    // The reasoning fallback calls token_counter(text=...) with no model, which also lands on cl100k_base.
    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.encode_len(text)
    }

    // token_counter(messages=None) raises in LiteLLM and the builder counts 0, so an error or an Ok(0) answer is the same count.
    #[cfg_attr(test, mutants::skip)]
    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[CountMessage]>,
    ) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }
}

impl ReasoningTokenCounter for CollectedCounter {
    type Failure = EngineError;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, EngineError> {
        self.engine.encode(text).map(|ids| ids.len())
    }
}
