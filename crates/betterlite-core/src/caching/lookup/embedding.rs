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
use super::json::{embedding_entries_cache_value, kwargs_json};
use super::response::LookupResponse;
use super::services::ResponseTokens;

// LiteLLM's _process_async_embedding_cached_response, minus the S3 cache, which has no bulk writes
pub async fn partial_embedding_hit<L, C, Conv>(
    pass: &mut HitPass<'_, L, C, Conv>,
    entries: Vec<Option<CachedEmbedding>>,
    logging: HitLogging,
) -> Result<AsyncLookup<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink,
    C: ConsoleSink,
    Conv: ReplayConversions + Sync,
{
    if pass.services.cache_type == LiteLlmCacheType::S3 {
        let raw = embedding_entries_cache_value(&entries)
            .map_err(|source| LookupError::Render { source })?;
        return Ok(AsyncLookup {
            response: Some(LookupResponse {
                cached_result: Some(Replayed::Unchanged(raw)),
                ..LookupResponse::default()
            }),
            logging: Some(logging),
        });
    }
    merge_cached_embeddings(pass, &entries, logging).await
}

async fn merge_cached_embeddings<L, C, Conv>(
    pass: &mut HitPass<'_, L, C, Conv>,
    entries: &[Option<CachedEmbedding>],
    logging: HitLogging,
) -> Result<AsyncLookup<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink,
    C: ConsoleSink,
    Conv: ReplayConversions + Sync,
{
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
    let returned = services.take_logging()?;
    Ok(AsyncLookup {
        response: Some(LookupResponse {
            final_embedding_response: outcome.response,
            all_embeddings_cached: outcome.all_elements_cached,
            embedding_uncached_input: Some(uncached),
            ..LookupResponse::default()
        }),
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
    // None when a complete hit already queued the callback job with the logging state
    pub fn take_logging(&self) -> Result<Option<HitLogging>, HitError> {
        let mut logging = self.logging.lock().map_err(|_poison| HitError::Poisoned)?;
        Ok(logging.take())
    }

    fn complete_hit(&self, logging: HitLogging, hit: &CompleteHit<'_>) -> Result<(), HitError> {
        let provider = resolve_provider(self.resolution, hit.model, hit.kwargs)?;
        let kwargs = kwargs_json(hit.kwargs);
        let response = embedding_response_json(hit.response);
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
        let Some(logging) = self.take_logging()? else {
            return Ok(());
        };
        self.complete_hit(logging, hit)
    }
}

fn embedding_response_json(response: &CachedEmbeddingResponse) -> Json {
    Json::Object(IndexMap::from([
        (
            "model".to_owned(),
            response
                .model
                .as_ref()
                .map_or(Json::Null, |model| Json::Text(model.clone())),
        ),
        (
            "data".to_owned(),
            Json::List(response.data.iter().map(embedding_json).collect()),
        ),
        (
            "usage".to_owned(),
            response.usage.as_ref().map_or(Json::Null, usage_json),
        ),
        (
            "hidden_params".to_owned(),
            Json::Object(response.hidden_params.clone()),
        ),
        (
            "response_headers".to_owned(),
            response
                .response_headers
                .as_ref()
                .map_or(Json::Null, |headers| Json::Object(headers.clone())),
        ),
    ]))
}

fn embedding_json(slot: &Option<Embedding>) -> Json {
    let Some(item) = slot else {
        return Json::Null;
    };
    Json::Object(IndexMap::from([
        ("embedding".to_owned(), payload_json(&item.embedding)),
        ("index".to_owned(), Json::Number(Number::from(item.index))),
        ("object".to_owned(), Json::Text("embedding".to_owned())),
    ]))
}

fn payload_json(payload: &EmbeddingPayload) -> Json {
    match payload {
        EmbeddingPayload::Vector(values) => {
            Json::List(values.iter().cloned().map(Json::Number).collect())
        }
        EmbeddingPayload::Text(text) => Json::Text(text.clone()),
    }
}

fn usage_json(usage: &Usage) -> Json {
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
            usage
                .prompt_tokens_details
                .as_ref()
                .map_or(Json::Null, |details| Json::Object(details.clone())),
        ),
    ]))
}
