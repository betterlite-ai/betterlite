pub mod default_encoding;
pub mod error;
pub mod messages;
pub mod request;
pub mod settings;
mod tests;
pub mod text;
pub mod wire;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_models::catalog::ModelCatalog;

use crate::engine::Engine;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::limits::{BuildJob, CountingService};
use crate::engine::selection::Selector;
use crate::engine::tiktoken::{Encoding, TiktokenEngine};
use crate::selection::openai_tokenizer_encoding_name;

use error::CountError;
use request::{SelectedTokenizer, TextInput, TokenRequest};
use settings::Settings;
use text::{Counted, Texter};

pub struct Counter {
    selector: Arc<Selector>,
    fetcher: Arc<TiktokenFetcher>,
    service: Arc<CountingService>,
    catalog: Arc<ModelCatalog>,
    settings: Settings,
    encodings: Mutex<BTreeMap<Encoding, Engine>>,
}

impl Counter {
    #[must_use]
    pub const fn new(
        selector: Arc<Selector>,
        fetcher: Arc<TiktokenFetcher>,
        service: Arc<CountingService>,
        catalog: Arc<ModelCatalog>,
        settings: Settings,
    ) -> Self {
        Self {
            selector,
            fetcher,
            service,
            catalog,
            settings,
            encodings: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn count(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        if self.settings.disabled {
            return Ok(0);
        }
        if request.text.is_some() && request.messages.is_some() {
            return Err(CountError::TextAndMessages);
        }
        match request.text {
            Some(text) => self.count_text(request, text).await,
            None => match request.messages {
                Some(messages) => self.count_message_tokens(request, messages).await,
                None => Err(CountError::MissingInput),
            },
        }
    }

    async fn count_text(
        &self,
        request: &TokenRequest<'_>,
        text: &TextInput,
    ) -> Result<usize, CountError> {
        if request.tools.is_some_and(|tools| !tools.is_empty()) || request.tool_choice.is_some() {
            return Err(CountError::ToolsWithText);
        }
        let counted = self.counted(request).await?;
        let texter = Texter {
            counter: self.service.as_ref(),
            counted: &counted,
            max_exact_chars: self.settings.max_exact_chars,
        };
        texter.count(&text.joined()).await
    }

    async fn count_message_tokens(
        &self,
        request: &TokenRequest<'_>,
        messages: &[request::Message],
    ) -> Result<usize, CountError> {
        let counted = self.counted(request).await?;
        let texter = Texter {
            counter: self.service.as_ref(),
            counted: &counted,
            max_exact_chars: self.settings.max_exact_chars,
        };
        let params = messages::message_params(request.model, &self.catalog);
        let mut total = messages::count_messages(&texter, params, messages).await?;
        if request.count_response_tokens == Some(false) {
            let includes_system = messages.iter().any(|message| message.role == "system");
            let extra =
                messages::count_extra(&texter, request.tools, request.tool_choice, includes_system)
                    .await?;
            total = total.checked_add(extra).ok_or(CountError::Overflow)?;
        }
        Ok(total)
    }

    async fn counted(&self, request: &TokenRequest<'_>) -> Result<Counted, CountError> {
        let selected = if let Some(tokenizer) = request.custom_tokenizer {
            tokenizer.clone()
        } else {
            let (engine, kind) = self.selector.select(request.model, None).await?;
            SelectedTokenizer::from_selection(engine, kind)
        };
        match selected {
            // an openai handle carries no encoding: LiteLLM re-derives it from the model name
            SelectedTokenizer::OpenAi => {
                let encoding = openai_tokenizer_encoding_name(request.model, &self.catalog);
                Ok(Counted {
                    engine: self.encoding_engine(encoding).await?,
                    chunks: Some(self.settings.chunk_chars),
                })
            }
            SelectedTokenizer::HuggingFace(engine) => Ok(Counted {
                engine,
                chunks: None,
            }),
        }
    }

    async fn encoding_engine(&self, encoding: Encoding) -> Result<Engine, CountError> {
        if let Some(found) = self
            .encodings
            .lock()
            .ok()
            .and_then(|cache| cache.get(&encoding).cloned())
        {
            return Ok(found);
        }
        let data = self.fetcher.data(encoding).await?;
        let job: BuildJob = Box::new(move || {
            let engine = TiktokenEngine::from_bpe_file(encoding, &data)?;
            Ok(Engine::Tiktoken(Arc::new(engine)))
        });
        let engine = self.service.build(job).await?;
        if let Ok(mut cache) = self.encodings.lock() {
            cache.insert(encoding, engine.clone());
        }
        Ok(engine)
    }
}
