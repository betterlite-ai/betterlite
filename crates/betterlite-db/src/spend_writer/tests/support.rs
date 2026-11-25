#![cfg(test)]

use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;

use super::accounting::{
    AdmissionFuture, SpendAccounting, SpendAccountingAdmission, SpendAccountingClosed,
};
use super::request::{PayloadFuture, SpendPayloadSource};
use super::{SpendWriterFailure, SpendWriterFailureSink, SpendWriterOperation};
use crate::daily_records::callers::{DailySpendQueue, QueueFuture};
use crate::daily_records::projection::{CallTypeEndpoints, DailyPricing};
use crate::spend_attribution::enqueue::{EnqueueFuture, EntitySpendQueue};
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::DailySpendUpdate;
use crate::spend_queues::entity::SpendUpdateQueueItem;

// A scripted sink: it records what the writer charged and fails the enqueue at the chosen position.
pub(crate) struct ScriptedQueue<T> {
    items: Mutex<Vec<T>>,
    fail_at: Option<usize>,
    failed: AtomicBool,
}

impl<T> ScriptedQueue<T> {
    pub(crate) fn new(fail_at: Option<usize>) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            fail_at,
            failed: AtomicBool::new(false),
        }
    }

    fn push(&self, item: T) -> Result<(), SpendQueueError> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        if self.fail_at == Some(items.len()) && !self.failed.swap(true, Ordering::SeqCst) {
            return Err(SpendQueueError::ZeroCapacity);
        }
        items.push(item);
        drop(items);
        Ok(())
    }

    pub(crate) fn items(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// The entity sink over the scripted queue.
pub(crate) struct RecordingEntityQueue {
    queue: ScriptedQueue<SpendUpdateQueueItem>,
}

impl RecordingEntityQueue {
    pub(crate) fn new(fail_at: Option<usize>) -> Self {
        Self {
            queue: ScriptedQueue::new(fail_at),
        }
    }

    pub(crate) fn items(&self) -> Vec<SpendUpdateQueueItem> {
        self.queue.items()
    }
}

impl EntitySpendQueue for RecordingEntityQueue {
    fn enqueue(&self, update: SpendUpdateQueueItem) -> EnqueueFuture<'_> {
        Box::pin(ready(self.queue.push(update)))
    }
}

// The daily sink over the same scripted queue.
pub(crate) struct RecordingDailyQueue {
    queue: ScriptedQueue<DailySpendUpdate>,
}

impl RecordingDailyQueue {
    pub(crate) fn new(fail_at: Option<usize>) -> Self {
        Self {
            queue: ScriptedQueue::new(fail_at),
        }
    }

    pub(crate) fn updates(&self) -> Vec<DailySpendUpdate> {
        self.queue.items()
    }
}

impl DailySpendQueue for RecordingDailyQueue {
    fn enqueue(&self, update: DailySpendUpdate) -> QueueFuture<'_> {
        Box::pin(ready(self.queue.push(update)))
    }
}

// The bounded handoff a request test drives: it keeps the owned work items, or refuses them like a closed queue.
pub(crate) struct RecordingAdmission {
    refused: bool,
    work: Mutex<Vec<SpendAccounting>>,
}

impl RecordingAdmission {
    pub(crate) const fn admitting() -> Self {
        Self {
            refused: false,
            work: Mutex::new(Vec::new()),
        }
    }

    pub(crate) const fn closed() -> Self {
        Self {
            refused: true,
            work: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn work(&self) -> Vec<SpendAccounting> {
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendAccountingAdmission for RecordingAdmission {
    fn admit(&self, work: SpendAccounting) -> AdmissionFuture<'_> {
        Box::pin(async move {
            if self.refused {
                return Err(SpendAccountingClosed);
            }
            self.work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(work);
            Ok(())
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedFailure {
    pub(crate) operation: SpendWriterOperation,
    pub(crate) token: Option<String>,
}

pub(crate) struct RecordingFailures {
    reported: Mutex<Vec<RecordedFailure>>,
}

impl RecordingFailures {
    pub(crate) const fn new() -> Self {
        Self {
            reported: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reported(&self) -> Vec<RecordedFailure> {
        self.reported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendWriterFailureSink for RecordingFailures {
    fn report(&self, failure: SpendWriterFailure<'_>) {
        self.reported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedFailure {
                operation: failure.operation,
                token: failure.token.map(ToOwned::to_owned),
            });
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the scripted spend payload is unavailable")]
pub(crate) struct PayloadFailure;

pub(crate) enum PayloadScript {
    Bytes(&'static str),
    Failure,
}

pub(crate) struct ScriptedPayload {
    pub(crate) script: PayloadScript,
}

impl SpendPayloadSource for ScriptedPayload {
    type Error = PayloadFailure;

    fn build(&self) -> PayloadFuture<'_, PayloadFailure> {
        let outcome: Result<Vec<u8>, PayloadFailure> = match &self.script {
            PayloadScript::Bytes(text) => Ok(text.as_bytes().to_vec()),
            PayloadScript::Failure => Err(PayloadFailure),
        };
        Box::pin(ready(outcome))
    }
}

// A writer's pricing state with no public rate behind it, so a daily row prices at zero instead of failing.
pub(crate) struct NoPrices;

impl PriceLookup for NoPrices {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

impl SavingsPricingContext for NoPrices {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            model: model.to_owned(),
            provider: custom_llm_provider.unwrap_or_default().to_owned(),
        })
    }

    fn deployment_price(
        &self,
        _deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        ready(None)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self
    }
}

pub(crate) struct NoEndpoints;

impl CallTypeEndpoints for NoEndpoints {
    fn endpoint(&self, _call_type: &str) -> Option<&str> {
        None
    }
}

pub(crate) struct Case {
    pub(crate) prices: NoPrices,
    pub(crate) zone: TimeZoneDatabase,
    pub(crate) endpoints: NoEndpoints,
}

impl Case {
    pub(crate) fn new() -> Self {
        Self {
            prices: NoPrices,
            zone: TimeZoneDatabase::none(),
            endpoints: NoEndpoints,
        }
    }

    pub(crate) fn pricing(&self) -> DailyPricing<'_, NoPrices> {
        DailyPricing {
            context: &self.prices,
            zone: &self.zone,
            fallback_moment: DateTime::UNIX_EPOCH,
        }
    }
}
