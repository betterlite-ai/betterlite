use std::convert::Infallible;
use std::future::{Future, ready};
use std::mem;
use std::sync::PoisonError;

use betterlite_core::api::batch_enqueued_refund::{
    BATCH_ENQUEUED_REFUND_STATUSES, BatchEnqueuedRefundInput, BatchEnqueuedRefundResult,
    PostCallOutcome,
};
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, ReservationBackend,
};
use betterlite_core::api::batch_input_accounting::error::BatchHttpError;
use betterlite_core::api::batch_limit_counters::records::{
    BatchFileUsage, BatchTpdRefundOperation,
};
use betterlite_core::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use betterlite_core::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use betterlite_core::api::batch_precall::ports::{
    BatchAccountRequest, BatchCacheError, BatchPrecallMemory, BatchPrecallSkip, BatchPrecallStages,
    BatchPrecallStash, BatchRefundSink, BatchTokenApplier,
};
use betterlite_core::api::request_destination::RequestData;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;
use thiserror::Error;

use super::{Fault, Function, Scripted};

mod remote;

// the stage failure the source catches: a bare exception, or one its caller re-raises as an HTTPException
#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct StageError {
    pub(crate) message: String,
    pub(crate) http: Option<BatchHttpError>,
}

impl BatchPrecallStages for Scripted {
    type Error = StageError;

    fn should_skip(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        _has_enqueued_scopes: bool,
    ) -> Result<BatchPrecallSkip, StageError> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push("should_skip");
        Ok(BatchPrecallSkip {
            skip: self.stages.skip,
            descriptors: None,
        })
    }

    fn account(
        &self,
        _principal: &BatchLimitPrincipal,
        _request: &BatchAccountRequest<'_>,
    ) -> impl Future<Output = Result<BatchFileUsage, StageError>> + Send {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push("account");
        ready(Ok(self.stages.usage.clone()))
    }

    fn reserve(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StageError>> + Send {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push("reserve");
        if self.stages.reject {
            return ready(Ok(Some(self.rejection.clone())));
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.held = Some(BatchEnqueuedTokenReservation {
            tokens: usage.total_tokens,
            scopes: scopes.to_vec(),
            backend: self.stages.backend.unwrap_or(ReservationBackend::Memory),
            owner: self.stages.owner.clone(),
            reserved_at_monotonic: 0.0_f64,
        });
        drop(state);
        ready(Ok(None))
    }

    fn charge(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        _usage: &BatchFileUsage,
        _descriptors: Option<&[BatchLimitDescriptor]>,
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StageError>> + Send {
        ready(Ok(charge_answer(self)))
    }

    fn http(&self, error: &StageError) -> Option<BatchHttpError> {
        error.http.clone()
    }
}

impl BatchPrecallStash for Scripted {
    fn take_reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .held
            .take()
    }

    fn take_refund_operations(&self) -> Vec<BatchTpdRefundOperation> {
        mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .operations,
        )
    }
}

impl BatchRefundSink for Scripted {
    type Error = StageError;

    fn handle(
        &self,
        input: &BatchEnqueuedRefundInput,
    ) -> impl Future<Output = Result<BatchEnqueuedRefundResult, StageError>> + Send {
        ready(self.refund_failure.as_ref().map_or_else(
            || Ok(refund_answer(input)),
            |message| {
                Err(StageError {
                    message: message.clone(),
                    http: None,
                })
            },
        ))
    }
}

fn refund_answer(input: &BatchEnqueuedRefundInput) -> BatchEnqueuedRefundResult {
    match input {
        BatchEnqueuedRefundInput::Refund { .. } => BatchEnqueuedRefundResult::Refunded,
        BatchEnqueuedRefundInput::PostCall {
            batch_id,
            status,
            pending,
        } => BatchEnqueuedRefundResult::PostCall(PostCallOutcome {
            batch_id: batch_id.clone(),
            saved: pending.is_some(),
            refunded: pending.is_some()
                && BATCH_ENQUEUED_REFUND_STATUSES.contains(&status.to_lowercase().as_str()),
        }),
        BatchEnqueuedRefundInput::Save { .. } => BatchEnqueuedRefundResult::Saved,
        BatchEnqueuedRefundInput::Pop { .. } => BatchEnqueuedRefundResult::Popped(None),
    }
}

impl BatchTokenApplier for Scripted {
    type Error = Infallible;

    fn apply(
        &self,
        _operations: &[BatchTpdRefundOperation],
    ) -> impl Future<Output = Result<(), Infallible>> + Send {
        ready(Ok(()))
    }
}

impl BatchPrecallMemory for Scripted {
    fn keys(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .memory
            .keys()
            .cloned()
            .collect()
    }

    fn get(&self, key: &str) -> Result<Option<Json>, BatchCacheError> {
        let value = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .memory
            .get(key)
            .cloned();
        cache_error(&self.memory_faults, Function::Get).map_or_else(|| Ok(value), Err)
    }

    fn set(
        &self,
        key: &str,
        value: Json,
        _ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        let fault = cache_error(&self.memory_faults, Function::Set);
        if let Some(error) = fault {
            return ready(Err(error));
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .memory
            .insert(key.to_owned(), value);
        ready(Ok(()))
    }

    fn set_many(
        &self,
        entries: Vec<(String, Json)>,
        _ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        match cache_error(&self.memory_faults, Function::SetMany) {
            Some(error) => ready(Err(error)),
            None => {
                self.state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .memory
                    .extend(entries);
                ready(Ok(()))
            }
        }
    }
}

// the counters stage: record the call, then the scripted refusal or a pass
fn charge_answer(scripted: &Scripted) -> Option<Box<ProxyRateLimitError>> {
    scripted
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .effects
        .push("charge");
    scripted.stages.reject.then(|| scripted.rejection.clone())
}

fn cache_error(faults: &[Fault], function: Function) -> Option<BatchCacheError> {
    let fault = faults.iter().find(|fault| fault.function == function)?;
    Some(fault.status.map_or_else(
        || BatchCacheError::Failed {
            message: fault.message.clone(),
        },
        |status| BatchCacheError::Http {
            status,
            detail: Json::Text(fault.message.clone()),
        },
    ))
}
