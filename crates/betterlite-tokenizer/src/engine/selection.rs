use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use betterlite_models::catalog::ModelCatalog;
use lru::LruCache;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::engine::Engine;
use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::fetch::{FetchError, TiktokenFetcher};
use crate::engine::hub::{HubDownloader, HubError};
use crate::engine::huggingface::HuggingFaceEngine;
use crate::engine::tiktoken::Encoding;

const CUSTOM_CACHE_SIZE: usize = 64;
const COHERE_REPO: &str = "Xenova/c4ai-command-r-v01-tokenizer";
const LLAMA2_REPO: &str = "hf-internal-testing/llama-tokenizer";
const LLAMA3_REPO: &str = "Xenova/llama-3-tokenizer";
const DEFAULT_REVISION: &str = "main";

#[derive(Debug, Clone)]
pub struct CustomTokenizer {
    pub identifier: String,
    pub revision: String,
    pub auth_token: Option<SecretString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerType {
    OpenAi,
    HuggingFace,
}

#[derive(Debug, Clone)]
pub struct SelectTokenizerResponse {
    pub tokenizer_type: TokenizerType,
    pub engine: Engine,
}

#[derive(Debug, Error)]
pub enum SelectionError {
    #[error("the custom tokenizer could not be downloaded")]
    Hub(#[from] HubError),
    #[error("the bundled tokenizer could not be fetched")]
    Fetch(#[from] FetchError),
    #[error("the tokenizer could not be built")]
    Engine(#[from] EngineError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Cohere,
    Anthropic,
    Llama2,
    Llama3,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CustomKey {
    identifier: String,
    revision: String,
    token: Option<String>,
}

pub struct Selector {
    fetcher: Arc<TiktokenFetcher>,
    hub: Arc<HubDownloader>,
    catalog: Arc<ModelCatalog>,
    disable_hf_tokenizer_download: bool,
    kinds: Mutex<BTreeMap<Kind, Engine>>,
    openai: Mutex<Option<Engine>>,
    custom: Mutex<LruCache<CustomKey, Engine>>,
}

impl Selector {
    #[must_use]
    pub fn new(
        fetcher: Arc<TiktokenFetcher>,
        hub: Arc<HubDownloader>,
        catalog: Arc<ModelCatalog>,
        disable_hf_tokenizer_download: bool,
    ) -> Self {
        let capacity = NonZeroUsize::new(CUSTOM_CACHE_SIZE).unwrap_or(NonZeroUsize::MIN);
        Self {
            fetcher,
            hub,
            catalog,
            disable_hf_tokenizer_download,
            kinds: Mutex::new(BTreeMap::new()),
            openai: Mutex::new(None),
            custom: Mutex::new(LruCache::new(capacity)),
        }
    }

    pub async fn select(
        &self,
        model: &str,
        custom: Option<&CustomTokenizer>,
    ) -> Result<SelectTokenizerResponse, SelectionError> {
        match custom {
            Some(tokenizer) => self.select_custom(tokenizer).await,
            None => self.select_default(model).await,
        }
    }

    async fn select_custom(
        &self,
        custom: &CustomTokenizer,
    ) -> Result<SelectTokenizerResponse, SelectionError> {
        let key = CustomKey {
            identifier: custom.identifier.clone(),
            revision: custom.revision.clone(),
            token: custom
                .auth_token
                .as_ref()
                .map(|token| hex::encode(Sha256::digest(token.expose_secret().as_bytes()))),
        };
        if let Some(found) = self.cached_custom(&key) {
            return Ok(SelectTokenizerResponse {
                tokenizer_type: TokenizerType::HuggingFace,
                engine: found,
            });
        }
        tracing::debug!(
            "Loading custom HuggingFace tokenizer {} (revision {})",
            custom.identifier,
            custom.revision
        );
        let bytes = self
            .hub
            .download(
                &custom.identifier,
                &custom.revision,
                custom.auth_token.as_ref(),
            )
            .await?;
        let engine = Engine::HuggingFace(Arc::new(HuggingFaceEngine::from_bytes(&bytes)?));
        if let Ok(mut cache) = self.custom.lock() {
            cache.put(key, engine.clone());
        }
        Ok(SelectTokenizerResponse {
            tokenizer_type: TokenizerType::HuggingFace,
            engine,
        })
    }

    fn cached_custom(&self, key: &CustomKey) -> Option<Engine> {
        match self.custom.lock() {
            Ok(mut cache) => cache.get(key).cloned(),
            Err(_poisoned) => None,
        }
    }

    async fn select_default(&self, model: &str) -> Result<SelectTokenizerResponse, SelectionError> {
        if self.disable_hf_tokenizer_download {
            return self.openai_response().await;
        }
        let Some(kind) = self.kind(model) else {
            return self.openai_response().await;
        };
        match self.load_kind(kind).await {
            Ok(engine) => Ok(SelectTokenizerResponse {
                tokenizer_type: TokenizerType::HuggingFace,
                engine,
            }),
            Err(error) => {
                tracing::debug!("Error selecting tokenizer: {error}");
                self.openai_response().await
            }
        }
    }

    async fn openai_response(&self) -> Result<SelectTokenizerResponse, SelectionError> {
        if let Some(found) = self
            .openai
            .lock()
            .ok()
            .and_then(|cache| cache.as_ref().cloned())
        {
            return Ok(SelectTokenizerResponse {
                tokenizer_type: TokenizerType::OpenAi,
                engine: found,
            });
        }
        let engine = Engine::Tiktoken(self.fetcher.engine(Encoding::Cl100kBase).await?);
        if let Ok(mut cache) = self.openai.lock() {
            *cache = Some(engine.clone());
        }
        Ok(SelectTokenizerResponse {
            tokenizer_type: TokenizerType::OpenAi,
            engine,
        })
    }

    async fn load_kind(&self, kind: Kind) -> Result<Engine, SelectionError> {
        if let Some(found) = self
            .kinds
            .lock()
            .ok()
            .and_then(|cache| cache.get(&kind).cloned())
        {
            return Ok(found);
        }
        let repository = match kind {
            Kind::Anthropic => None,
            Kind::Cohere => Some(COHERE_REPO),
            Kind::Llama2 => Some(LLAMA2_REPO),
            Kind::Llama3 => Some(LLAMA3_REPO),
        };
        let engine = match repository {
            Some(id) => self.hub_engine(id).await?,
            None => Engine::HuggingFace(Arc::new(HuggingFaceEngine::from_bytes(&assets::joined(
                assets::ANTHROPIC_PARTS,
            ))?)),
        };
        if let Ok(mut cache) = self.kinds.lock() {
            cache.insert(kind, engine.clone());
        }
        Ok(engine)
    }

    async fn hub_engine(&self, repository: &str) -> Result<Engine, SelectionError> {
        match self.hub.download(repository, DEFAULT_REVISION, None).await {
            Ok(bytes) => Ok(Engine::HuggingFace(Arc::new(
                HuggingFaceEngine::from_bytes(&bytes)?,
            ))),
            Err(error) => Err(SelectionError::Hub(error)),
        }
    }

    fn kind(&self, model: &str) -> Option<Kind> {
        if self.provider_is(model, "cohere") && model.contains("command-r") {
            return Some(Kind::Cohere);
        }
        if self.provider_is(model, "anthropic") && !model.contains("claude-3") {
            return Some(Kind::Anthropic);
        }
        let lowered = model.to_lowercase();
        if lowered.contains("llama-2") || lowered.contains("replicate") {
            return Some(Kind::Llama2);
        }
        if lowered.contains("llama-3") {
            return Some(Kind::Llama3);
        }
        None
    }

    fn provider_is(&self, model: &str, provider: &str) -> bool {
        self.catalog
            .entries
            .get(model)
            .is_some_and(|entry| entry.fields.provider_projection().litellm_provider == provider)
    }
}
