use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::catalog::ModelCatalog;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::messages::{count_extra, count_messages, message_params};
use betterlite_tokenizer::count::request::Message;
use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::count::text::{Counted, Texter};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::tiktoken::TiktokenEngine;
use betterlite_tokenizer::selection::openai_tokenizer_encoding_name;
use serde_json::Value;
use thiserror::Error;

// LiteLLM's token_counter(model=..., messages=...): an embedding count of every message plus the reply priming
#[async_trait]
pub trait PromptTokens: Send + Sync {
    async fn count(&self, model: &str, messages: &[Value]) -> Result<usize, PromptCountError>;
}

#[derive(Debug, Error)]
pub enum PromptCountError {
    #[error("the prompt messages are not chat messages: {source}")]
    Messages {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite ships no {encoding} vocabulary, so the prompt cannot be counted")]
    Unbundled { encoding: &'static str },
    #[error("the prompt tokens could not be counted: {source}")]
    Count {
        #[from]
        source: CountError,
    },
    #[error("the tokenizer could not be built: {source}")]
    Engine {
        #[from]
        source: EngineError,
    },
}

// the recovery path needs one count and owns no counting pool, so the encoding runs inline
pub struct TokenizerPromptTokens;

#[async_trait]
impl PromptTokens for TokenizerPromptTokens {
    async fn count(&self, model: &str, messages: &[Value]) -> Result<usize, PromptCountError> {
        let settings = Settings::default();
        if settings.disabled {
            return Ok(0);
        }
        let parsed: Vec<Message> = serde_json::from_value(Value::Array(messages.to_vec()))
            .map_err(|source| PromptCountError::Messages { source })?;
        let catalog = ModelCatalog::default();
        let encoding = openai_tokenizer_encoding_name(model, &catalog);
        let vocabulary = assets::bundled(encoding).ok_or(PromptCountError::Unbundled {
            encoding: encoding.name(),
        })?;
        let engine = TiktokenEngine::from_bpe_file(encoding, &vocabulary)?;
        let service = CountingService::new(1, Arc::new(InlineCounter), settings.encode_limits()?);
        let counted = Counted {
            engine: Engine::Tiktoken(Arc::new(engine)),
            chunks: Some(settings.chunk_chars),
        };
        let texter = Texter {
            counter: &service,
            counted: &counted,
            max_exact_chars: settings.max_exact_chars,
        };
        let params = message_params(model, &catalog);
        let mut total = count_messages(&texter, params, &parsed).await?;
        // no tool definitions are counted here, so the system-message discount of count_extra never applies
        total = total
            .checked_add(count_extra(&texter, None, None, false).await?)
            .ok_or(CountError::Overflow)?;
        Ok(total)
    }
}

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    // the recovery path counts through CountingService::count only, so no test can reach a build or convert job
    #[cfg_attr(test, mutants::skip)]
    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    #[cfg_attr(test, mutants::skip)]
    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}
