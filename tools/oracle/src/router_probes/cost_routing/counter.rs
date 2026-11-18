use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::count::text::{Counted, Texter};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

// litellm.token_counter(text=...) with the empty model the selector passes: cl100k with the bundled vocabulary
pub(crate) struct ProbeText {
    service: CountingService,
    counted: Counted,
    max_exact_chars: NonZeroUsize,
}

impl ProbeText {
    pub(crate) fn new() -> Result<Self, String> {
        let settings = Settings::default();
        let data = assets::bundled(Encoding::Cl100kBase)
            .ok_or_else(|| "the cl100k vocabulary is not bundled".to_owned())?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data)
            .map_err(|error| error.to_string())?;
        let limits = settings
            .encode_limits()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            service: CountingService::new(1, Arc::new(InlineCounter), limits),
            counted: Counted {
                engine: Engine::Tiktoken(Arc::new(engine)),
                chunks: Some(settings.chunk_chars),
            },
            max_exact_chars: settings.max_exact_chars,
        })
    }

    pub(crate) async fn count(&self, text: &str) -> Result<usize, CountError> {
        let texter = Texter {
            counter: &self.service,
            counted: &self.counted,
            max_exact_chars: self.max_exact_chars,
        };
        texter.count(text).await
    }
}
