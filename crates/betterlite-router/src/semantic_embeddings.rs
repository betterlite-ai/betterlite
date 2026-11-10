pub mod error;
pub mod redis;
mod tests;

use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_tokenizer::engine::selection::Selector;
use tokio::time::timeout;

use self::error::SemanticEmbeddingError;
// ast-grep-ignore: no-long-names
use crate::semantic_embedding_inputs::build_router_embedding_metadata as router_metadata;
// ast-grep-ignore: no-long-names
use crate::semantic_embedding_inputs::resolve_embedding_max_input_tokens as max_tokens_for;
use crate::semantic_embedding_inputs::{
    ConfiguredTokenLimits, MetadataMap, ModelListEntry, resolve_embedding_router,
    resolve_embedding_timeout, truncate_embedding_input,
};

pub struct SemanticEmbeddingSettings {
    pub embedding_model: String,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: Option<f64>,
    pub default_timeout_seconds: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingRoute {
    Router,
    Direct,
}

// LiteLLM's sync embedding method returns without the wait_for wrapper, its async one applies it
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingMode {
    Sync,
    Async,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmbeddingCachePolicy {
    pub no_store: bool,
    pub no_cache: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingRequest {
    pub route: EmbeddingRoute,
    pub model: String,
    pub input: String,
    pub cache: EmbeddingCachePolicy,
    pub metadata: Option<MetadataMap>,
    pub timeout: f64,
    pub num_retries: u32,
}

pub trait EmbeddedReply {
    fn first_embedding(&self) -> Option<Vec<f64>>;
}

pub trait EmbeddingFailure: Error + Send + Sync + 'static {
    fn status_code(&self) -> Option<u16>;
}

pub trait EmbeddingCalls: Send + Sync {
    type Response: EmbeddedReply + Send;
    type Error: EmbeddingFailure;

    fn embedding(
        &self,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;

    fn aembedding(
        &self,
        request: &EmbeddingRequest,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

pub struct SemanticEmbeddingService<R, C> {
    embedding_model: String,
    embedding_max_input_tokens: Option<i64>,
    embedding_timeout: f64,
    router: Option<R>,
    model_list: Option<Vec<ModelListEntry>>,
    calls: C,
    selector: Arc<Selector>,
}

impl<R, C> SemanticEmbeddingService<R, C>
where
    R: ConfiguredTokenLimits + Sync,
    C: EmbeddingCalls,
{
    #[must_use]
    pub fn new(
        settings: SemanticEmbeddingSettings,
        router: Option<R>,
        model_list: Option<Vec<ModelListEntry>>,
        calls: C,
        selector: Arc<Selector>,
    ) -> Self {
        Self {
            embedding_model: settings.embedding_model,
            embedding_max_input_tokens: settings.embedding_max_input_tokens,
            embedding_timeout: resolve_embedding_timeout(
                settings.embedding_timeout,
                settings.default_timeout_seconds,
            ),
            router,
            model_list,
            calls,
            selector,
        }
    }

    pub async fn call(
        &self,
        request: &EmbeddingRequest,
        mode: EmbeddingMode,
    ) -> Result<C::Response, SemanticEmbeddingError<C::Error>> {
        match mode {
            EmbeddingMode::Sync => self
                .calls
                .embedding(request)
                .await
                .map_err(SemanticEmbeddingError::Call),
            EmbeddingMode::Async => {
                // asyncio.wait_for refuses a timeout at or below zero without ever running the call
                if self.embedding_timeout <= 0.0_f64 || self.embedding_timeout.is_nan() {
                    return Err(SemanticEmbeddingError::Timeout {
                        seconds: self.embedding_timeout,
                    });
                }
                // an infinite or oversized delay never fires in asyncio, so the call runs unbounded
                let Ok(wait) = Duration::try_from_secs_f64(self.embedding_timeout) else {
                    return self
                        .calls
                        .aembedding(request)
                        .await
                        .map_err(SemanticEmbeddingError::Call);
                };
                match timeout(wait, self.calls.aembedding(request)).await {
                    Ok(result) => result.map_err(SemanticEmbeddingError::Call),
                    Err(_elapsed) => Err(SemanticEmbeddingError::Timeout {
                        seconds: self.embedding_timeout,
                    }),
                }
            }
        }
    }

    fn resolved_max_input_tokens(&self, router: Option<&R>) -> Option<i64> {
        router.map_or_else(
            || max_tokens_for(self.embedding_max_input_tokens, &self.embedding_model, None),
            |found| {
                max_tokens_for(
                    self.embedding_max_input_tokens,
                    &self.embedding_model,
                    Some(found),
                )
            },
        )
    }

    pub async fn prepare(
        &self,
        prompt: &str,
        metadata: Option<&MetadataMap>,
    ) -> Result<EmbeddingRequest, SemanticEmbeddingError<C::Error>> {
        let router = resolve_embedding_router(
            &self.embedding_model,
            self.router.as_ref(),
            self.model_list.as_deref(),
        );
        let max_input_tokens = self.resolved_max_input_tokens(router);
        let input = truncate_embedding_input(
            &self.selector,
            prompt,
            &self.embedding_model,
            max_input_tokens,
        )
        .await
        .map_err(SemanticEmbeddingError::Input)?;
        Ok(EmbeddingRequest {
            route: router.map_or(EmbeddingRoute::Direct, |_| EmbeddingRoute::Router),
            model: self.embedding_model.clone(),
            input,
            cache: EmbeddingCachePolicy {
                no_store: true,
                no_cache: true,
            },
            metadata: router.map(|_| router_metadata(metadata)),
            timeout: self.embedding_timeout,
            num_retries: 0,
        })
    }
}
