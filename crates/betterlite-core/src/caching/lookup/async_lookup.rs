use betterlite_cache::handler::retrieve::{Retrieval, retrieve_from_cache};
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::replay::{
    CacheHit, ReplayConversions, Replayed, ReplayedBy, replay_cached_result,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};

use super::eligibility::{async_cache_enabled, cache_supports_call};
use super::embedding::partial_embedding_hit;
use super::error::LookupError;
use super::hit::{
    CacheTiming, HitKind, HitLogging, HitPass, HitResponse, HitTimes, cache_duration,
    hit_cache_key, hit_record, log_converted_hit,
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

// LiteLLM's _async_get_cache: the eligibility checks run before any expensive operation
pub async fn async_get_cache<L, C, Conv>(
    handler: &mut CachingHandler,
    request: CacheLookup,
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
        return Ok(AsyncLookup {
            response: None,
            logging: Some(logging),
        });
    }
    if !cache_supports_call(
        services.cache.is_some(),
        &handler.original_function.name,
        services.supported_call_types,
    ) {
        return Ok(AsyncLookup {
            response: Some(LookupResponse::default()),
            logging: Some(logging),
        });
    }
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
    let timing = CacheTiming {
        started,
        finished: (services.perf)(),
    };
    let mut pass = HitPass {
        handler,
        request: &request,
        kwargs: &kwargs,
        services,
        conversions,
    };
    let value = match retrieval {
        Retrieval::Miss => {
            return Ok(AsyncLookup {
                response: Some(LookupResponse::default()),
                logging: Some(logging),
            });
        }
        Retrieval::Embeddings(entries) => {
            return partial_embedding_hit(&mut pass, entries, logging).await;
        }
        // a cached list is never a single response, so LiteLLM hands it back as it was stored
        Retrieval::Hit(value @ CacheValue::Array(_)) => {
            return Ok(AsyncLookup {
                response: Some(LookupResponse::cached(Replayed::Unchanged(value))),
                logging: Some(logging),
            });
        }
        Retrieval::Hit(value) => value,
    };
    let end_time = pass.services.wall.now();
    let record = hit_record(
        pass.services.resolution,
        &pass.request.model,
        pass.kwargs,
        &value,
        Some(cache_duration(timing)),
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
