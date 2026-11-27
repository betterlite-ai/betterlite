use std::time::Duration;

use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::value::KeyObject;
use betterlite_cache::handler::kwargs::{key_request, request_cache_key};
use betterlite_cache::handler::retrieve::CacheSource;
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::replay::{CacheHitMetadata, ReplayConversions, Replayed};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
// ast-grep-ignore: no-long-names
use betterlite_logging::logging_success::dispatch::handle_sync_success_callbacks_for_async_calls as dispatch_sync_callbacks;
use betterlite_logging::logging_success::handler::SuccessHandler;
use betterlite_logging::logging_success::owned::OwnedSuccessHandler;
use betterlite_logging::logging_success::types::{SuccessRequest, SuccessResult};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_logging::worker::{Admission, LoggingWorker};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::caching::hit_callbacks::context::{CacheHitContext, record_cache_hit_context};
use crate::caching::hit_callbacks::job::{
    CacheHitCallbacks, CacheHitSuccessJob, log_cache_hit_on_callbacks,
};

use super::error::{HitError, LookupError};
use super::json::{cache_value_json, kwargs_json, request_text};
use super::request::CacheLookup;
use super::services::LookupServices;

// The logging object's cache-hit half: the success handler, the state it logs into and the correlation ids
pub struct HitLogging {
    pub handler: Arc<OwnedSuccessHandler>,
    pub state: LoggingState,
    pub correlation: CorrelationContext,
}

#[derive(Clone, Copy, Debug)]
pub struct HitTimes {
    pub start: Option<DateTime<FixedOffset>>,
    pub end: DateTime<FixedOffset>,
}
// The two perf_counter reads around the cache retrieval, which the logged cache duration is measured from
#[derive(Clone, Copy, Debug)]
pub struct CacheTiming {
    pub started: Duration,
    pub finished: Duration,
}
// Which entry point records the hit, and the two flags its environment update reads
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitKind {
    AsyncResponse,
    AsyncEmbedding,
    SyncResponse,
}

pub struct Provider {
    pub model: String,
    pub provider: String,
}

// The resolved provider and the rendered request and result a hit records, owned so the hit context can borrow them
pub struct HitRecord {
    pub provider: Provider,
    pub kwargs: IndexMap<String, Json>,
    pub cached_result: Json,
    pub cache_duration_ms: Option<f64>,
}

// One cache lookup's working set, shared by the async and the sync hit paths
pub struct HitPass<'pass, L: LoggerSink, C: ConsoleSink, Conv: ReplayConversions> {
    pub handler: &'pass mut CachingHandler,
    pub request: &'pass CacheLookup,
    pub kwargs: &'pass KeyObject,
    pub services: &'pass LookupServices<'pass, L, C>,
    pub conversions: &'pass Conv,
}

// What a converted cached response must offer the hit path: the stamped cache key and the result the callbacks log
pub trait HitResponse {
    fn stamp_cache_key(&mut self, cache_key: Option<&str>);
    fn success_result(&self) -> SuccessResult;
}

impl<Response> HitResponse for Replayed<Response>
where
    Response: HitResponse,
{
    fn stamp_cache_key(&mut self, cache_key: Option<&str>) {
        match self {
            Self::Response(_, response)
            | Self::Transcription(response)
            | Self::Responses(response)
            | Self::Stream(response)
            | Self::ResponsesStream(response)
            | Self::MessagesStream(response) => response.stamp_cache_key(cache_key),
            // a raw cached value has no hidden parameters to stamp
            Self::Unchanged(_) => {}
        }
    }

    fn success_result(&self) -> SuccessResult {
        match self {
            Self::Response(_, response)
            | Self::Transcription(response)
            | Self::Responses(response)
            | Self::Stream(response)
            | Self::ResponsesStream(response)
            | Self::MessagesStream(response) => response.success_result(),
            Self::Unchanged(value) => SuccessResult::Other(cache_value_json(value)),
        }
    }
}

impl HitRecord {
    #[must_use]
    pub fn context<'record>(
        &'record self,
        model: &'record str,
        preset_cache_key: Option<&'record str>,
        kind: HitKind,
        cached_result: &'record Json,
    ) -> CacheHitContext<'record> {
        CacheHitContext {
            model,
            custom_llm_provider: Some(self.provider.provider.as_str()),
            preset_cache_key,
            cache_duration_ms: self.cache_duration_ms,
            kwargs: &self.kwargs,
            cached_result,
            is_async: kind != HitKind::SyncResponse,
            is_embedding: kind == HitKind::AsyncEmbedding,
        }
    }
}

pub fn hit_record(
    resolution: &ResolutionContext<'_>,
    model: &str,
    kwargs: &KeyObject,
    cached_result: &CacheValue,
    cache_duration_ms: Option<f64>,
) -> Result<HitRecord, LookupError> {
    Ok(HitRecord {
        provider: resolve_provider(resolution, model, kwargs)?,
        kwargs: kwargs_json(kwargs),
        cached_result: cache_value_json(cached_result),
        cache_duration_ms,
    })
}

pub fn log_hit_async(
    worker: &LoggingWorker<CacheHitSuccessJob>,
    hit: &CacheHitContext<'_>,
    result: SuccessResult,
    times: HitTimes,
    logging: HitLogging,
) -> Result<Admission, HitError> {
    let callbacks = CacheHitCallbacks {
        handler: logging.handler,
        state: logging.state,
        correlation: logging.correlation,
        result,
        start_time: times.start,
        end_time: Some(times.end),
    };
    Ok(log_cache_hit_on_callbacks(worker, hit, callbacks)?)
}

// A converted hit is logged here unless a replayed stream logs it when the stream finishes
pub fn log_converted_hit<Response>(
    replayed: &mut Replayed<Response>,
    hit: &CacheHitContext<'_>,
    times: HitTimes,
    mut logging: HitLogging,
    worker: &LoggingWorker<CacheHitSuccessJob>,
) -> Result<Option<HitLogging>, HitError>
where
    Response: HitResponse + CacheHitMetadata,
{
    if replayed.defers_success_callbacks() {
        record_cache_hit_context(&mut logging.state, hit)?;
        return Ok(Some(logging));
    }
    let result = replayed.success_result();
    let _admission = log_hit_async(worker, hit, result, times, logging)?;
    Ok(None)
}

// The sync path runs the callbacks on the logging object itself instead of queueing a job
pub fn log_hit_sync(
    hit: &CacheHitContext<'_>,
    result: SuccessResult,
    times: HitTimes,
    logging: &mut HitLogging,
) -> Result<(), HitError> {
    record_cache_hit_context(&mut logging.state, hit)?;
    let success = SuccessHandler::from(logging.handler.as_ref());
    let request = SuccessRequest {
        result,
        start_time: times.start,
        end_time: Some(times.end),
        cache_hit: Some(true),
        prefer_async_handlers: false,
        standard_logging_object: None,
        guardrail_snapshot: None,
    };
    dispatch_sync_callbacks(
        &success,
        &mut logging.state,
        &mut logging.correlation,
        &request,
    );
    Ok(())
}

// preset_cache_key or the request's cache_key or the one computed from the request kwargs; any of them may be None
pub fn hit_cache_key<L, C>(
    handler: &CachingHandler,
    cache: Option<&CacheSource<'_, L, C>>,
) -> Result<Option<String>, LookupError>
where
    L: LoggerSink,
    C: ConsoleSink,
{
    if let Some(preset) = &handler.preset_cache_key {
        return Ok(Some(preset.clone()));
    }
    if let Some(key) = request_cache_key(&handler.request_kwargs)? {
        return Ok(Some(key));
    }
    let Some(source) = cache else {
        return Ok(None);
    };
    let mut request = key_request(&handler.request_kwargs)?;
    let key = get_cache_key(
        &mut request,
        &source.cache.key_settings,
        &source.cache.hooks,
    )?;
    Ok(Some(key))
}

// LiteLLM's litellm.get_llm_provider for a cache hit: the resolved model and provider go into the logged environment
pub fn resolve_provider(
    resolution: &ResolutionContext<'_>,
    model: &str,
    kwargs: &KeyObject,
) -> Result<Provider, HitError> {
    let custom_llm_provider = request_text(kwargs, "custom_llm_provider")?;
    let api_base = request_text(kwargs, "api_base")?;
    let api_key = request_text(kwargs, "api_key")?.map(|key| SecretString::from(key.to_owned()));
    let request = ResolutionRequest {
        model,
        custom_llm_provider,
        api_base,
        api_key: api_key.as_ref(),
    };
    let resolved =
        resolve(&request, resolution).map_err(|source| HitError::Resolution { source })?;
    Ok(Provider {
        model: resolved.model,
        provider: resolved.provider.slug().to_owned(),
    })
}
