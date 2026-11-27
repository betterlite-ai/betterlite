use std::convert::Infallible;
use std::future::{Future, ready};
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
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push("should_skip");
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
        _: &BatchLimitPrincipal,
        _: &RequestData,
        usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StageError>> + Send {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push("reserve");
        if self.stages.reject {
            return ready(Ok(Some(self.rejection.clone())));
        }
        state.held = Some(BatchEnqueuedTokenReservation {
            tokens: usage.total_tokens,
            scopes: scopes.to_vec(),
            backend: self.stages.backend.unwrap_or(ReservationBackend::Memory),
            owner: self.stages.owner.clone(),
            reserved_at_monotonic: 0.0_f64,
        });
        ready(Ok(None))
    }

    fn charge(
        &self,
        _: &BatchLimitPrincipal,
        _: &RequestData,
        _: &BatchFileUsage,
        _: Option<&[BatchLimitDescriptor]>,
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StageError>> + Send {
        ready(charge_answer(self))
    }

    fn http(&self, error: &StageError) -> Option<&BatchHttpError> {
        error.http.as_ref()
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
        std::mem::take(
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

    fn run(
        &self,
        input: &BatchEnqueuedRefundInput,
    ) -> impl Future<Output = Result<BatchEnqueuedRefundResult, StageError>> + Send {
        let outcome = match &self.refund_failure {
            Some(message) => Err(StageError {
                message: message.clone(),
                http: None,
            }),
            None => Ok(refund_answer(input)),
        };
        ready(outcome)
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
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let value = state.memory.get(key).cloned();
        match cache_error(&self.memory_faults, Function::Get) {
            Some(error) => Err(error),
            None => Ok(value),
        }
    }

    fn set(
        &self,
        key: &str,
        value: Json,
        _ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        if let Some(error) = cache_error(&self.memory_faults, Function::Set) {
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
fn charge_answer(scripted: &Scripted) -> Result<Option<Box<ProxyRateLimitError>>, StageError> {
    let mut state = scripted
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state.effects.push("charge");
    Ok(if scripted.stages.reject {
        Some(scripted.rejection.clone())
    } else {
        None
    })
}

fn cache_error(faults: &[Fault], function: Function) -> Option<BatchCacheError> {
    let fault = faults.iter().find(|fault| fault.function == function)?;
    Some(match fault.status {
        Some(status) => BatchCacheError::Http {
            status,
            detail: Json::Text(fault.message.clone()),
        },
        None => BatchCacheError::Failed {
            message: fault.message.clone(),
        },
    })
}
