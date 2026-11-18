use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::ports::{
    CacheWrite, DynamicCacheObject, FailureDelivery, PortFailure, ResponsesStreamCacheHandler,
    ResponsesStreamCacheTarget, ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use thiserror::Error;

use super::{CacheMode, CostMode, Input, TransformMode};

pub(crate) struct Probe {
    pub(crate) transform: TransformMode,
    cost: CostMode,
    cache: CacheMode,
    records: Mutex<Vec<Value>>,
    pub(crate) clock: DateTime<FixedOffset>,
    kwargs: IndexMap<String, Json>,
}

impl Probe {
    pub(crate) fn new(input: &Input, clock: DateTime<FixedOffset>) -> Self {
        let mut kwargs = IndexMap::new();
        kwargs.insert("stream".to_owned(), Json::Bool(true));
        Self {
            transform: input.transform,
            cost: input.cost,
            cache: input.cache,
            records: Mutex::new(Vec::new()),
            clock,
            kwargs,
        }
    }

    pub(crate) fn record(&self, value: Value) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }

    pub(crate) fn records(&self) -> Vec<Value> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SuccessClock for Probe {
    fn now(&self) -> DateTime<FixedOffset> {
        self.clock
    }
}

pub(crate) fn hooks<'context>(
    probe: &Arc<Probe>,
    redactor: &'context Redactor,
) -> ResponsesStreamHooks<'context> {
    ResponsesStreamHooks {
        cache: Some(Arc::<Probe>::clone(probe)),
        cache_handler: Some(Arc::<Probe>::clone(probe)),
        success: Some(Arc::<Probe>::clone(probe)),
        failure: Some(Arc::<Probe>::clone(probe)),
        metadata: None,
        success_hook: None,
        chunk_hooks: Vec::new(),
        clock: Arc::<Probe>::clone(probe),
        redactor,
    }
}

impl ResponsesStreamCacheHandler for Probe {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>> {
        match self.cache {
            CacheMode::Off => None,
            CacheMode::Failing => Some(&self.kwargs),
        }
    }

    fn preset_cache_key(&self) -> Option<&str> {
        None
    }

    fn original_function(&self) -> Option<&str> {
        Some("aresponses")
    }

    fn dual_cache(&self) -> Option<DynamicCacheObject> {
        None
    }

    fn should_store_result_in_cache(
        &self,
        _original_function: Option<&str>,
        _kwargs: &IndexMap<String, Json>,
    ) -> Result<bool, PortFailure> {
        Ok(true)
    }
}

#[async_trait]
impl ResponsesStreamCacheTarget for Probe {
    fn add_cache(&self, _write: CacheWrite) -> Result<(), PortFailure> {
        Err(PortFailure {
            message: "cache write failed".to_owned(),
        })
    }

    async fn async_add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.add_cache(write)
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for Probe {
    async fn dispatch_success_handlers(&self, _request: SuccessRequest) {
        self.record(Value::String("success_dispatch".to_owned()));
    }

    async fn async_success_handler(&self, _request: &SuccessRequest) {
        self.record(Value::String("success_async".to_owned()));
    }

    fn success_handler(&self, _request: &SuccessRequest) {
        self.record(Value::String("success_sync".to_owned()));
    }

    fn deferred_armed(&self) -> bool {
        false
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for Probe {
    async fn async_failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(Value::String("failure_async".to_owned()));
        Ok(())
    }

    fn failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(Value::String("failure_sync".to_owned()));
        Ok(())
    }
}

pub(crate) struct Calculator {
    probe: Arc<Probe>,
}

impl Calculator {
    pub(crate) fn new(probe: Arc<Probe>) -> Self {
        Self { probe }
    }
}

impl ResponsesCostCalculator for Calculator {
    type Error = CostFailure;

    fn response_cost(&self, target: &CostTarget<'_>) -> Result<Option<Decimal>, CostFailure> {
        let id = match target {
            CostTarget::Model(response) => Json::Text(response.id.clone()),
            CostTarget::Object(fields) => fields.get("id").cloned().unwrap_or(Json::Null),
        };
        self.probe.record(json!(["cost", id]));
        match self.probe.cost {
            CostMode::Positive => Ok(Some(Decimal::new(5, 1))),
            CostMode::Zero => Ok(Some(Decimal::ZERO)),
            CostMode::Missing => Ok(None),
            CostMode::Error => Err(CostFailure),
        }
    }
}

#[derive(Debug, Error)]
#[error("cost calculation failed")]
pub(crate) struct CostFailure;
