use std::ops::Mul as _;

use betterlite_cache::cache_key::value::KeyObject;
use betterlite_cache::handler::call_types::call_type_supported;
use betterlite_cache::handler::retrieve::{Retrieval, retrieve_from_cache};
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::replay::{
    CacheHit, ReplayConversions, Replayed, ReplayedBy, replay_cached_result,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};

use super::eligibility::async_cache_enabled;
use super::embedding::partial_embedding_hit;
use super::error::LookupError;
use super::hit::{
    CacheTiming, HitKind, HitLogging, HitPass, HitResponse, HitTimes, hit_cache_key, hit_record,
    log_converted_hit,
};
use super::parent_span::add_parent_otel_span;
use super::request::{CacheLookup, replay_request};
use super::response::LookupResponse;
use super::services::LookupServices;

// None means LiteLLM's _async_get_cache returned None, so the caller runs the call itself
pub struct AsyncLookup<Result> {
    pub response: Option<LookupResponse<Result>>,
    // Some when no callback job took the logging state, so the caller still logs the call
    pub logging: Option<HitLogging>,
}

impl<Result> AsyncLookup<Result> {
    const fn new(response: Option<LookupResponse<Result>>, logging: HitLogging) -> Self {
        Self {
            response,
            logging: Some(logging),
        }
    }
}

// LiteLLM's _async_get_cache: the eligibility checks run before any expensive operation
pub async fn async_get_cache<L, C, Conv>(
    handler: &mut CachingHandler,
    request: &CacheLookup,
    logging: HitLogging,
    services: &LookupServices<'_, L, C>,
    conversions: &Conv,
) -> Result<AsyncLookup<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
    Conv: ReplayConversions + Sync,
    Conv::Response: HitResponse,
{
    if !async_cache_enabled(&request.kwargs, services.cache.is_some())? {
        return Ok(AsyncLookup::new(None, logging));
    }
    let supports_call = services.cache.is_some()
        && call_type_supported(
            &handler.original_function.name,
            services.supported_call_types,
        );
    if !supports_call {
        return Ok(AsyncLookup::new(Some(LookupResponse::default()), logging));
    }
    let (kwargs, retrieval, timing) = async_fetch(handler, request, services).await?;
    let mut pass = HitPass {
        handler,
        request,
        kwargs: &kwargs,
        services,
        conversions,
    };
    match retrieval {
        Retrieval::Miss => Ok(AsyncLookup::new(None, logging)),
        Retrieval::Embeddings(entries) => partial_embedding_hit(&mut pass, &entries, logging).await,
        // a cached list is never a single response, so LiteLLM hands it back as it was stored
        Retrieval::Hit(value @ CacheValue::Array(_)) => Ok(AsyncLookup::new(
            Some(LookupResponse::cached(Replayed::Unchanged(value))),
            logging,
        )),
        Retrieval::Hit(value) => async_hit(&pass, value, timing, logging),
    }
}

async fn async_fetch<L, C>(
    handler: &mut CachingHandler,
    request: &CacheLookup,
    services: &LookupServices<'_, L, C>,
) -> Result<(KeyObject, Retrieval, CacheTiming), LookupError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let started = (services.perf)();
    let mut kwargs = request.kwargs.clone();
    add_parent_otel_span(&mut kwargs);
    let retrieval = retrieve_from_cache(
        handler,
        services.cache,
        request.call_type,
        &kwargs,
        &request.args,
    )
    .await?;
    let finished = (services.perf)();
    Ok((kwargs, retrieval, CacheTiming { started, finished }))
}

// A full hit: convert the stored value, record the hit's context and log the converted result
fn async_hit<L, C, Conv>(
    pass: &HitPass<'_, L, C, Conv>,
    value: CacheValue,
    timing: CacheTiming,
    logging: HitLogging,
) -> Result<AsyncLookup<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink,
    C: ConsoleSink,
    Conv: ReplayConversions + Sync,
    Conv::Response: HitResponse,
{
    let end_time = pass.services.wall.now();
    let elapsed = timing.finished.saturating_sub(timing.started);
    let record = hit_record(
        pass.services.resolution,
        &pass.request.model,
        pass.kwargs,
        &value,
        Some(elapsed.as_secs_f64().mul(1000.0_f64)),
    )?;
    let replay = replay_request(pass.request, pass.kwargs, &record.provider.model, end_time)?;
    let mut replayed = replay_cached_result(
        CacheHit::Stored(value),
        pass.request.call_type,
        &replay,
        pass.conversions,
    )?;
    let hit = record.context(
        &record.provider.model,
        pass.handler.preset_cache_key.as_deref(),
        HitKind::AsyncResponse,
        &record.cached_result,
    );
    let times = HitTimes {
        start: Some(pass.request.start_time),
        end: end_time,
    };
    let returned = log_converted_hit(&mut replayed, &hit, times, logging, pass.services.worker)?;
    let cache_key = hit_cache_key(pass.handler, pass.services.cache)?;
    replayed.stamp_cache_key(cache_key.as_deref());
    Ok(AsyncLookup {
        response: Some(LookupResponse::cached(replayed)),
        logging: returned,
    })
}
