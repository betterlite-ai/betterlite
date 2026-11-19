use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_cache::embedding_merge::cached::{
    CacheHitServices, CachedLookup, CompleteHit, process_cached_embeddings,
};
use betterlite_cache::embedding_merge::response::{CachedEmbeddingResponse, Embedding};
use betterlite_cache::embedding_merge::usage::Usage;
use betterlite_cache::handler::embedding::input_items;
use betterlite_cache::replay::{ReplayConversions, Replayed, ReplayedBy};
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::embedding::{CachedEmbedding, EmbeddingPayload};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_logging::worker::LoggingWorker;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use chrono::Utc;
use indexmap::IndexMap;
use serde_json::Number;

use crate::caching::hit_callbacks::context::CacheHitContext;
use crate::caching::hit_callbacks::job::CacheHitSuccessJob;

use super::async_lookup::AsyncLookup;
use super::error::{HitError, LookupError};
use super::hit::{HitLogging, HitPass, HitTimes, log_hit_async, resolve_provider};
use super::json::{cache_value_of, kwargs_json};
use super::response::LookupResponse;
use super::services::ResponseTokens;

// LiteLLM's _process_async_embedding_cached_response, minus the S3 cache, which has no bulk writes
pub async fn partial_embedding_hit<L, C, Conv>(
    pass: &mut HitPass<'_, L, C, Conv>,
    entries: &[Option<CachedEmbedding>],
    logging: HitLogging,
) -> Result<AsyncLookup<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink,
    C: ConsoleSink,
    Conv: ReplayConversions + Sync,
{
    if pass.services.cache_type == LiteLlmCacheType::S3 {
        let raw = cache_value_of(entries)?;
        return Ok(AsyncLookup {
            response: Some(LookupResponse::cached(Replayed::Unchanged(raw))),
            logging: Some(logging),
        });
    }
    let lookup = CachedLookup {
        cached_result: entries,
        kwargs: &mut pass.handler.request_kwargs,
        model: &pass.request.model,
        start_time: pass.request.start_time.with_timezone(&Utc),
    };
    let services = EmbeddingHitServices {
        worker: pass.services.worker,
        resolution: pass.services.resolution,
        tokens: pass.services.tokens,
        wall: pass.services.wall.as_ref(),
        logging: Mutex::new(Some(logging)),
    };
    let outcome = process_cached_embeddings(lookup, None, &services).await?;
    let uncached = input_items(&pass.handler.request_kwargs)?;
    let returned = {
        let mut held = services
            .logging
            .lock()
            .map_err(|_poison| HitError::Poisoned)?;
        held.take()
    };
    Ok(AsyncLookup {
        response: Some(LookupResponse::merged(
            outcome.response,
            outcome.all_elements_cached,
            uncached,
        )),
        logging: returned,
    })
}
// What B4303's cached embedding merge needs from the gateway, holding the hit's logging object until the merge finishes
pub struct EmbeddingHitServices<'svc> {
    pub worker: &'svc LoggingWorker<CacheHitSuccessJob>,
    pub resolution: &'svc ResolutionContext<'svc>,
    pub tokens: &'svc dyn ResponseTokens,
    pub wall: &'svc dyn SuccessClock,
    pub logging: Mutex<Option<HitLogging>>,
}

impl EmbeddingHitServices<'_> {
    fn complete_hit(&self, logging: HitLogging, hit: &CompleteHit<'_>) -> Result<(), HitError> {
        let provider = resolve_provider(self.resolution, hit.model, hit.kwargs)?;
        let kwargs = kwargs_json(hit.kwargs);
        let response = embedding_json(hit.response);
        let context = CacheHitContext {
            model: &provider.model,
            custom_llm_provider: Some(provider.provider.as_str()),
            preset_cache_key: None,
            cache_duration_ms: None,
            kwargs: &kwargs,
            cached_result: &response,
            is_async: true,
            is_embedding: true,
        };
        let times = HitTimes {
            start: Some(hit.start_time.fixed_offset()),
            end: self.wall.now(),
        };
        let result = SuccessResult::Embeddings(response.clone());
        let _admission = log_hit_async(self.worker, &context, result, times, logging)?;
        Ok(())
    }
}

#[async_trait]
impl CacheHitServices for EmbeddingHitServices<'_> {
    type Error = HitError;

    async fn count_response_tokens(&self, text: &str) -> Result<i64, Self::Error> {
        Ok(self.tokens.count_response_tokens(text).await?)
    }

    fn log_complete_hit(&self, hit: &CompleteHit<'_>) -> Result<(), HitError> {
        let mut held = self.logging.lock().map_err(|_poison| HitError::Poisoned)?;
        let Some(logging) = held.take() else {
            return Ok(());
        };
        self.complete_hit(logging, hit)
    }
}
// The merged response as LiteLLM's EmbeddingResponse dict, which the hit's callbacks log
fn embedding_json(response: &CachedEmbeddingResponse) -> Json {
    let model = response.model.clone().map_or(Json::Null, Json::Text);
    let headers = response
        .response_headers
        .as_ref()
        .map_or(Json::Null, |headers| Json::Object(headers.clone()));
    let hidden = Json::Object(response.hidden_params.clone());
    Json::Object(IndexMap::from([
        ("model".to_owned(), model),
        ("data".to_owned(), embedding_data(&response.data)),
        ("usage".to_owned(), embedding_usage(response.usage.as_ref())),
        ("hidden_params".to_owned(), hidden),
        ("response_headers".to_owned(), headers),
    ]))
}

fn embedding_data(slots: &[Option<Embedding>]) -> Json {
    let items = slots
        .iter()
        .map(|slot| slot.as_ref().map_or(Json::Null, embedding_item));
    Json::List(items.collect())
}

fn embedding_item(item: &Embedding) -> Json {
    Json::Object(IndexMap::from([
        (
            "embedding".to_owned(),
            match &item.embedding {
                EmbeddingPayload::Vector(values) => {
                    Json::List(values.iter().cloned().map(Json::Number).collect())
                }
                EmbeddingPayload::Text(text) => Json::Text(text.clone()),
            },
        ),
        ("index".to_owned(), Json::Number(Number::from(item.index))),
        ("object".to_owned(), Json::Text("embedding".to_owned())),
    ]))
}

fn embedding_usage(usage: Option<&Usage>) -> Json {
    let Some(usage) = usage else {
        return Json::Null;
    };
    let details = usage.prompt_tokens_details.clone();
    Json::Object(IndexMap::from([
        (
            "prompt_tokens".to_owned(),
            Json::Number(Number::from(usage.prompt_tokens)),
        ),
        (
            "completion_tokens".to_owned(),
            Json::Number(Number::from(usage.completion_tokens)),
        ),
        (
            "total_tokens".to_owned(),
            Json::Number(Number::from(usage.total_tokens)),
        ),
        (
            "prompt_tokens_details".to_owned(),
            details.map_or(Json::Null, Json::Object),
        ),
    ]))
}
