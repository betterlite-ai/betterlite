use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_core::api::responses_stream_hooks::ports::{
    FailureDelivery, PortFailure, ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{CostMode, Input};

pub(crate) struct Probe {
    cost: CostMode,
    records: Mutex<Vec<Value>>,
    pub(crate) clock: DateTime<FixedOffset>,
}

impl Probe {
    pub(crate) fn new(input: &Input, clock: DateTime<FixedOffset>) -> Self {
        Self {
            cost: input.cost,
            records: Mutex::new(Vec::new()),
            clock,
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

impl ResponsesCostCalculator for Probe {
    type Error = Infallible;

    fn response_cost(&self, target: &CostTarget<'_>) -> Result<Option<Decimal>, Infallible> {
        let id = match target {
            CostTarget::Model(response) => Json::Text(response.id.clone()),
            CostTarget::Object(fields) => fields.get("id").cloned().unwrap_or(Json::Null),
        };
        self.record(json!(["cost", id]));
        match self.cost {
            CostMode::Positive => Ok(Some(Decimal::new(5, 1))),
            CostMode::Missing => Ok(None),
        }
    }
}
