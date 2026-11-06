use crate::engine::Engine;
use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::fetch::{FetchError, TiktokenFetcher};
use crate::engine::hub::{HubDownloader, HubError};
use crate::engine::huggingface::HuggingFaceEngine;
use crate::engine::tiktoken::Encoding;
use betterlite_models::catalog::ModelCatalog;
use lru::LruCache;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use thiserror::Error;
const CUSTOM_CACHE: usize = 64;
const DEFAULT_REVISION: &str = "main";
const COHERE_REPO: &str = "Xenova/c4ai-command-r-v01-tokenizer";
const LLAMA2_REPO: &str = "hf-internal-testing/llama-tokenizer";
const LLAMA3_REPO: &str = "Xenova/llama-3-tokenizer";
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
impl From<(Engine, TokenizerType)> for SelectTokenizerResponse {
    fn from((engine, tokenizer_type): (Engine, TokenizerType)) -> Self {
        Self {
            tokenizer_type,
            engine,
        }
    }
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
    OpenAi,
    Cohere,
    Anthropic,
    Llama2,
    Llama3,
}
type CustomKey = (String, String, Option<String>);
pub struct Selector {
    fetcher: Arc<TiktokenFetcher>,
    hub: Arc<HubDownloader>,
    catalog: Arc<ModelCatalog>,
    disable_hf_tokenizer_download: bool,
    kinds: Mutex<BTreeMap<Kind, Engine>>,
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
        let capacity = NonZeroUsize::new(CUSTOM_CACHE).unwrap_or(NonZeroUsize::MIN);
        Self {
            fetcher,
            hub,
            catalog,
            disable_hf_tokenizer_download,
            kinds: Mutex::new(BTreeMap::new()),
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
        let key: CustomKey = (
            custom.identifier.clone(),
            custom.revision.clone(),
            custom
                .auth_token
                .as_ref()
                .map(|token| hex::encode(Sha256::digest(token.expose_secret().as_bytes()))),
        );
        let cached = self
            .custom
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&key).cloned());
        if let Some(found) = cached {
            let response = SelectTokenizerResponse::from((found, TokenizerType::HuggingFace));
            return Ok(response);
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
        let response = SelectTokenizerResponse::from((engine, TokenizerType::HuggingFace));
        Ok(response)
    }
    async fn select_default(&self, model: &str) -> Result<SelectTokenizerResponse, SelectionError> {
        if !self.disable_hf_tokenizer_download
            && let Some(kind) = self.kind(model)
        {
            match self.load_kind(kind).await {
                Ok(engine) => {
                    let response =
                        SelectTokenizerResponse::from((engine, TokenizerType::HuggingFace));
                    return Ok(response);
                }
                Err(error) => tracing::debug!("Error selecting tokenizer: {error}"),
            }
        }
        self.default_encoding().await
    }
    pub async fn default_encoding(&self) -> Result<SelectTokenizerResponse, SelectionError> {
        Ok(SelectTokenizerResponse {
            tokenizer_type: TokenizerType::OpenAi,
            engine: self.load_kind(Kind::OpenAi).await?,
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
        let engine = match kind {
            Kind::OpenAi => Engine::Tiktoken(self.fetcher.engine(Encoding::Cl100kBase).await?),
            Kind::Anthropic => Engine::HuggingFace(Arc::new(HuggingFaceEngine::from_bytes(
                &assets::joined(assets::ANTHROPIC_PARTS),
            )?)),
            Kind::Cohere => self.download_kind(COHERE_REPO).await?,
            Kind::Llama2 => self.download_kind(LLAMA2_REPO).await?,
            Kind::Llama3 => self.download_kind(LLAMA3_REPO).await?,
        };
        if let Ok(mut cache) = self.kinds.lock() {
            cache.insert(kind, engine.clone());
        }
        Ok(engine)
    }
    async fn download_kind(&self, repository: &str) -> Result<Engine, SelectionError> {
        let bytes = self
            .hub
            .download(repository, DEFAULT_REVISION, None)
            .await?;
        let engine = HuggingFaceEngine::from_bytes(&bytes)?;
        Ok(Engine::HuggingFace(Arc::new(engine)))
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
