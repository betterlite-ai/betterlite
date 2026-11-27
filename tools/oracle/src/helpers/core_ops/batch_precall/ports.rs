use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_core::api::batch_enqueued_refund::{
    BATCH_ENQUEUED_REFUND_STATUSES, BatchEnqueuedRefundInput, BatchEnqueuedRefundResult,
    PostCallOutcome,
};
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope,
};
use betterlite_core::api::batch_input_accounting::error::BatchHttpError;
use betterlite_core::api::batch_limit_counters::records::{
    BatchFileUsage, BatchTpdRefundOperation,
};
use betterlite_core::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use betterlite_core::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use betterlite_core::api::batch_precall::ports::{
    BatchAccountRequest, BatchCacheError, BatchPrecallMemory, BatchPrecallRemote, BatchPrecallSkip,
    BatchPrecallStages, BatchPrecallStash, BatchRefundSink, BatchTokenApplier,
};
use betterlite_core::api::request_destination::RequestData;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;
use thiserror::Error;

use super::{Fault, Function, Scripted};

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
        self.record("should_skip");
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
        self.record("account");
        ready(Ok(self.stages.usage.clone()))
    }

    fn reserve(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StageError>> + Send {
        self.record("reserve");
        if self.stages.reject {
            return ready(Ok(Some(self.rejection.clone())));
        }
        self.hold(usage.total_tokens, scopes);
        ready(Ok(None))
    }

    fn charge(
        &self,
        _principal: &BatchLimitPrincipal,
        _data: &RequestData,
        _usage: &BatchFileUsage,
        _descriptors: Option<&[BatchLimitDescriptor]>,
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, StageError>> + Send {
        self.record("charge");
        let rejection = if self.stages.reject {
            Some(self.rejection.clone())
        } else {
            None
        };
        ready(Ok(rejection))
    }

    fn http(&self, error: &StageError) -> Option<&BatchHttpError> {
        error.http.as_ref()
    }
}

impl BatchPrecallStash for Scripted {
    fn take_reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        self.state().held.take()
    }

    fn take_refund_operations(&self) -> Vec<BatchTpdRefundOperation> {
        std::mem::take(&mut self.state().operations)
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
        self.state().memory.keys().cloned().collect()
    }

    fn get(&self, key: &str) -> Result<Option<Json>, BatchCacheError> {
        match cache_error(&self.memory_faults, Function::Get) {
            Some(error) => Err(error),
            None => Ok(self.state().memory.get(key).cloned()),
        }
    }

    fn set(
        &self,
        key: &str,
        value: Json,
        _ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        let outcome = match cache_error(&self.memory_faults, Function::Set) {
            Some(error) => Err(error),
            None => {
                self.state().memory.insert(key.to_owned(), value);
                Ok(())
            }
        };
        ready(outcome)
    }

    fn set_many(
        &self,
        entries: Vec<(String, Json)>,
        _ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send {
        let outcome = match cache_error(&self.memory_faults, Function::SetMany) {
            Some(error) => Err(error),
            None => {
                self.state().memory.extend(entries);
                Ok(())
            }
        };
        ready(outcome)
    }
}

impl BatchPrecallRemote for Scripted {
    fn is_redis(&self) -> bool {
        true
    }

    fn cache_key(&self, _data: &RequestData) -> Option<String> {
        None
    }

    fn scan(
        &self,
        _pattern: &str,
        _count: usize,
    ) -> impl Future<Output = Result<Vec<String>, BatchCacheError>> + Send {
        self.record("scan");
        let outcome = match cache_error(&self.remote.faults, Function::Scan) {
            Some(error) => Err(error),
            None => Ok(self.remote.scan_keys.clone()),
        };
        ready(outcome)
    }

    fn batch_get(
        &self,
        _keys: &[String],
    ) -> impl Future<Output = Result<Vec<(String, Json)>, BatchCacheError>> + Send {
        self.record("batch_get");
        let outcome = match cache_error(&self.remote.faults, Function::BatchGet) {
            Some(error) => Err(error),
            None => Ok(self
                .remote
                .batch_entries
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()),
        };
        ready(outcome)
    }

    fn get(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<Json>, BatchCacheError>> + Send {
        self.record("get");
        let outcome = match cache_error(&self.remote.faults, Function::Get) {
            Some(error) => Err(error),
            None => Ok(self.remote.get_value.clone()),
        };
        ready(outcome)
    }
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
