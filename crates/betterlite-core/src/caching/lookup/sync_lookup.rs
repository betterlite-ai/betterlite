use betterlite_cache::handler::retrieve::{
    AsyncSupport, CacheSource, Retrieval, retrieve_from_cache,
};
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::replay::{
    CacheHit, ReplayConversions, Replayed, ReplayedBy, replay_cached_result,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};

use crate::caching::hit_callbacks::context::record_cache_hit_context;

use super::eligibility::cache_supports_call;
use super::error::{HitError, LookupError};
use super::hit::{
    HitKind, HitLogging, HitPass, HitResponse, HitTimes, hit_cache_key, hit_record, log_hit_sync,
};
use super::request::{CacheLookup, replay_request};
use super::response::LookupResponse;
use super::services::LookupServices;

// LiteLLM's _sync_get_cache: only the call type gates the read, and Unsupported makes it the backend's own synchronous read
pub async fn sync_get_cache<L, C, Conv>(
    handler: &mut CachingHandler,
    request: CacheLookup,
    logging: &mut HitLogging,
    services: &LookupServices<'_, L, C>,
    conversions: &Conv,
) -> Result<LookupResponse<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
    Conv: ReplayConversions + Sync,
    Conv::Response: HitResponse,
{
    if !cache_supports_call(
        services.cache.is_some(),
        &handler.original_function.name,
        services.supported_call_types,
    ) {
        return Ok(LookupResponse::default());
    }
    let source = services.cache.map(|cache| CacheSource {
        cache: cache.cache,
        async_support: AsyncSupport::Unsupported,
    });
    let retrieval = retrieve_from_cache(
        handler,
        source.as_ref(),
        request.call_type,
        &request.kwargs,
        &request.args,
    )
    .await?;
    let mut pass = HitPass {
        handler,
        request: &request,
        kwargs: &request.kwargs,
        services,
        conversions,
    };
    match retrieval {
        Retrieval::Miss => Ok(LookupResponse::default()),
        Retrieval::Embeddings(entries) => {
            let raw =
                serde_json::to_value(&entries).map_err(|source| LookupError::Render { source })?;
            let raw: CacheValue =
                serde_json::from_value(raw).map_err(|source| LookupError::Render { source })?;
            Ok(LookupResponse::cached(Replayed::Unchanged(raw)))
        }
        // a cached error keeps its detail instead of being converted into a response
        Retrieval::Hit(value) if matches!(&value, CacheValue::Object(fields) if fields.contains_key("detail")) => {
            Ok(LookupResponse::cached(Replayed::Unchanged(value)))
        }
        Retrieval::Hit(value) => sync_hit(&mut pass, value, logging),
    }
}
fn sync_hit<L, C, Conv>(
    pass: &mut HitPass<'_, L, C, Conv>,
    value: CacheValue,
    logging: &mut HitLogging,
) -> Result<LookupResponse<ReplayedBy<Conv>>, LookupError>
where
    L: LoggerSink,
    C: ConsoleSink,
    Conv: ReplayConversions,
    Conv::Response: HitResponse,
{
    let end_time = pass.services.wall.now();
    let record = hit_record(
        pass.services.resolution,
        &pass.request.model,
        pass.kwargs,
        &value,
        None,
    )?;
    let replay = replay_request(pass.request, pass.kwargs, &pass.request.model, end_time)?;
    let mut replayed = replay_cached_result(
        CacheHit::Stored(value),
        pass.request.call_type,
        &replay,
        pass.conversions,
    )?;
    let model = format!("{}/{}", record.provider.provider, record.provider.model);
    // LiteLLM converts the sync hit before it updates the logging environment, so the recorded result is the converted one
    let converted = replayed.success_result();
    // the sync read never fills the handler's preset key, so the environment only reads the request's litellm_params
    let hit = record.context(&model, None, HitKind::SyncResponse, converted.response());
    let times = HitTimes {
        start: Some(pass.request.start_time),
        end: end_time,
    };
    if replayed.defers_success_callbacks() {
        record_cache_hit_context(&mut logging.state, &hit).map_err(|source| LookupError::Hit {
            source: HitError::Context { source },
        })?;
    } else {
        log_hit_sync(&hit, replayed.success_result(), times, logging)?;
    }
    let cache_key = hit_cache_key(pass.handler, pass.services.cache)?;
    replayed.stamp_cache_key(cache_key.as_deref());
    Ok(LookupResponse::cached(replayed))
}
