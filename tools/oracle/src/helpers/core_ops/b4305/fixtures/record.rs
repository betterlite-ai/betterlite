use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_cache::replay::{
    CacheHitMetadata, CachedDict, ReplayConversions, ReplayError, ReplayedBy, ResponseKind,
    TranscriptionHidden,
};
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_core::caching::hit_callbacks::context::CACHING_DETAILS;
use betterlite_core::caching::lookup::hit::HitResponse;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::{
    StandardPayloadBuilder, StreamAssembler, SuccessClock,
};
use betterlite_logging::logging_success::types::{SuccessMode, SuccessResult};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::handlers::helper_error;

// The logging object's cache-hit view after the call, which the Python helper reads off its Logging object
#[derive(Clone, Debug)]
pub(crate) struct Logged {
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) caching_details: Option<Json>,
}

impl Logged {
    pub(crate) fn from_state(state: &LoggingState) -> Self {
        Self {
            model: state.model.clone(),
            provider: state.custom_llm_provider.clone(),
            litellm_params: state.litellm_params.clone(),
            caching_details: state.model_call_details.get(CACHING_DETAILS).cloned(),
        }
    }
}

// A hit queues its logging on the worker, so the payload builder's snapshot is the state's only reader; a skipped log keeps the state here
pub(crate) fn logged_from(
    snapshots: &Mutex<Vec<Logged>>,
    fallback: Option<&LoggingState>,
) -> Result<Logged, Box<HelperError>> {
    let recorded = snapshots.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(snapshot) = recorded.last() {
        return Ok(snapshot.clone());
    }
    fallback.map_or_else(
        || {
            Err(helper_error(
                "the cache hit logging state was not recorded".to_owned(),
            ))
        },
        |state| Ok(Logged::from_state(state)),
    )
}

pub(crate) struct SnapshotBuilder {
    pub(crate) snapshots: Arc<Mutex<Vec<Logged>>>,
}

impl StandardPayloadBuilder for SnapshotBuilder {
    fn build(
        &self,
        state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Logged::from_state(state));
        None
    }
}

pub(crate) struct Assembler;

impl StreamAssembler for Assembler {
    fn assemble(
        &self,
        _state: &LoggingState,
        _result: &SuccessResult,
        _mode: SuccessMode,
    ) -> Option<Json> {
        None
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

// The converted hit records what the lookup asked of it: the cache-hit flag and the stamped key
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HelperResponse {
    pub(crate) cache_hit: bool,
    pub(crate) stamp: Option<Option<String>>,
}

impl CacheHitMetadata for HelperResponse {
    fn set_cache_hit(&mut self) {
        self.cache_hit = true;
    }
}

impl HitResponse for HelperResponse {
    fn stamp_cache_key(&mut self, cache_key: Option<&str>) {
        self.stamp = Some(cache_key.map(str::to_owned));
    }

    fn success_result(&self) -> SuccessResult {
        SuccessResult::Chat(Json::Bool(self.cache_hit))
    }
}

// A chat dictionary converts only when it carries choices, the shape LiteLLM's converter requires
pub(crate) struct Conversions;

impl ReplayConversions for Conversions {
    type Response = HelperResponse;

    fn model_response(
        &self,
        cached: &CachedDict,
        _kind: ResponseKind,
    ) -> Result<Self::Response, ReplayError> {
        if cached.contains_key("choices") {
            return Ok(HelperResponse::default());
        }
        Err(ReplayError::Shape {
            expected: "choices",
        })
    }

    fn transcription_response(
        &self,
        _cached: &CachedDict,
        _hidden: &TranscriptionHidden<'_>,
    ) -> Result<Self::Response, ReplayError> {
        Ok(HelperResponse::default())
    }

    fn responses_response(&self, _cached: &CachedDict) -> Result<Self::Response, ReplayError> {
        Ok(HelperResponse::default())
    }

    fn responses_stream(
        &self,
        response: Self::Response,
        _call_type: CachingSupportedCallType,
    ) -> Result<Self::Response, ReplayError> {
        Ok(response)
    }

    fn anthropic_messages_stream(
        &self,
        _cached: &CachedDict,
    ) -> Result<Option<Self::Response>, ReplayError> {
        Ok(None)
    }

    fn stream_wrapper(
        &self,
        _cached: &CachedDict,
        _asynchronous: bool,
        _model: &str,
        _custom_llm_provider: &str,
    ) -> Result<Self::Response, ReplayError> {
        Ok(HelperResponse::default())
    }

    fn update_response_metadata(
        &self,
        _result: &mut ReplayedBy<Self>,
        _model: &str,
        _elapsed: Duration,
    ) {
    }
}
