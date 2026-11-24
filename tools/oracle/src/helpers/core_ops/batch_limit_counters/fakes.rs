use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedReserveResult, BatchEnqueuedTokenOverLimit, BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope, ReservationBackend,
};
use betterlite_core::api::batch_limit_counters::ports::{
    BatchCounterLimiter, BatchEnqueuedSource, BatchWindowCache,
};
use betterlite_core::api::batch_limit_counters::records::{
    BatchCounterLimitType, BatchCounterResponse, WindowStartValue,
};
use betterlite_core::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use betterlite_core::api::batch_limit_descriptors::input::BatchModelUsage;
use betterlite_core::api::request_destination::RequestData;
use indexmap::IndexMap;
use serde_json::Value;

use super::scripts::{BackendName, CachedWindow, LimiterScript, StoreReply, StoreScript};
use super::views;

#[derive(Clone)]
pub(super) struct ScriptedStore {
    replies: Arc<Mutex<VecDeque<StoreReply>>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedStore {
    pub(super) fn new(script: &StoreScript) -> Self {
        Self {
            replies: Arc::new(Mutex::new(script.replies.iter().cloned().collect())),
            calls: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn reserved(
    tokens: i64,
    scopes: &[BatchEnqueuedTokenScope],
    backend: ReservationBackend,
    owner: String,
) -> BatchEnqueuedTokenReservation {
    BatchEnqueuedTokenReservation {
        tokens,
        scopes: scopes.to_vec(),
        backend,
        owner,
        reserved_at_monotonic: 0.0_f64,
    }
}

impl BatchEnqueuedSource for ScriptedStore {
    type Error = IoError;

    fn reserve(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<BatchEnqueuedReserveResult, Self::Error>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(views::store_call(tokens, scopes));
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let outcome = match next {
            None => Ok(BatchEnqueuedReserveResult::Reservation(reserved(
                tokens,
                scopes,
                ReservationBackend::Redis,
                String::new(),
            ))),
            Some(StoreReply::Reservation { backend, owner }) => {
                let chosen = backend.map_or(ReservationBackend::Memory, BackendName::reserved);
                Ok(BatchEnqueuedReserveResult::Reservation(reserved(
                    tokens,
                    scopes,
                    chosen,
                    owner.unwrap_or_default(),
                )))
            }
            Some(StoreReply::OverLimit { scope, enqueued }) => {
                Ok(BatchEnqueuedReserveResult::OverLimit(
                    BatchEnqueuedTokenOverLimit { scope, enqueued },
                ))
            }
        };
        ready(outcome)
    }
}

#[derive(Clone)]
pub(super) struct ScriptedCache {
    values: BTreeMap<String, Option<CachedWindow>>,
    reads: Arc<Mutex<Vec<String>>>,
}

impl ScriptedCache {
    pub(super) fn new(values: &BTreeMap<String, Option<CachedWindow>>) -> Self {
        Self {
            values: values.clone(),
            reads: Arc::default(),
        }
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BatchWindowCache for ScriptedCache {
    type Error = IoError;

    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<WindowStartValue>, Self::Error>> + Send {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        let stored = self
            .values
            .get(key)
            .cloned()
            .flatten()
            .map(CachedWindow::value);
        ready(Ok(stored))
    }
}

#[derive(Clone)]
pub(super) struct ScriptedLimiter {
    base: Vec<BatchLimitDescriptor>,
    project: (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>),
    responses: Arc<Mutex<VecDeque<BatchCounterResponse>>>,
    base_calls: Arc<Mutex<usize>>,
    project_calls: Arc<Mutex<usize>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedLimiter {
    pub(super) fn new(script: &LimiterScript) -> Self {
        Self {
            base: script.base.clone(),
            project: (
                script.project.descriptors.clone(),
                script.project.increments.clone(),
            ),
            responses: Arc::new(Mutex::new(script.responses.iter().cloned().collect())),
            base_calls: Arc::default(),
            project_calls: Arc::default(),
            calls: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn base_calls(&self) -> usize {
        *self
            .base_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn project_calls(&self) -> usize {
        *self
            .project_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl BatchCounterLimiter for ScriptedLimiter {
    type Error = IoError;

    fn base_descriptors(
        &self,
        _auth: &UserAPIKeyAuth,
        _data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, Self::Error> {
        let mut count = self
            .base_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_add(1);
        drop(count);
        Ok(self.base.clone())
    }

    fn project_io_descriptors(
        &self,
        _auth: &UserAPIKeyAuth,
        _per_model_usage: &IndexMap<String, BatchModelUsage>,
    ) -> (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>) {
        let mut count = self
            .project_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_add(1);
        drop(count);
        self.project.clone()
    }

    // ast-grep-ignore: no-long-names
    fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[BatchLimitDescriptor],
        increments: &[BatchDescriptorIncrements],
    ) -> impl Future<Output = Result<BatchCounterResponse, Self::Error>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(views::limiter_call(descriptors, increments));
        let answer = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| IoError::other("the scripted limiter has no response left"));
        ready(answer)
    }

    fn create_rate_limit_keys(
        &self,
        key: &str,
        value: &str,
        rate_limit_type: BatchCounterLimitType,
    ) -> String {
        format!("{{{key}:{value}}}:{}", rate_limit_type.as_str())
    }
}
