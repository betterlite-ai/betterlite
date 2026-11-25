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

use super::scripts::{BackendName, CachedWindow, StoreReply};
use super::views;

#[derive(Clone)]
pub(super) struct ScriptedStore {
    pub(super) replies: Arc<Mutex<VecDeque<StoreReply>>>,
    pub(super) calls: Arc<Mutex<Vec<views::StoreCallView>>>,
}

impl ScriptedStore {
    pub(super) fn calls(&self) -> Vec<views::StoreCallView> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BatchEnqueuedSource for ScriptedStore {
    type Error = IoError;

    fn reserve_tokens(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<BatchEnqueuedReserveResult, Self::Error>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(views::StoreCallView {
                tokens,
                scopes: scopes.to_vec(),
            });
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let outcome = match next {
            None => Ok(BatchEnqueuedReserveResult::Reservation(
                BatchEnqueuedTokenReservation {
                    tokens,
                    scopes: scopes.to_vec(),
                    backend: ReservationBackend::Redis,
                    owner: String::new(),
                    reserved_at_monotonic: 0.0_f64,
                },
            )),
            Some(StoreReply::Reservation { backend, owner }) => Ok(
                BatchEnqueuedReserveResult::Reservation(BatchEnqueuedTokenReservation {
                    tokens,
                    scopes: scopes.to_vec(),
                    backend: backend.map_or(ReservationBackend::Memory, BackendName::reserved),
                    owner: owner.unwrap_or_default(),
                    reserved_at_monotonic: 0.0_f64,
                }),
            ),
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
    pub(super) values: BTreeMap<String, Option<CachedWindow>>,
    pub(super) reads: Arc<Mutex<Vec<String>>>,
}

impl ScriptedCache {
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
    pub(super) base: Vec<BatchLimitDescriptor>,
    pub(super) project: (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>),
    pub(super) responses: Arc<Mutex<VecDeque<BatchCounterResponse>>>,
    pub(super) base_calls: Arc<Mutex<usize>>,
    pub(super) project_calls: Arc<Mutex<usize>>,
    pub(super) calls: Arc<Mutex<Vec<views::LimiterCallView>>>,
}

impl ScriptedLimiter {
    pub(super) fn calls(&self) -> Vec<views::LimiterCallView> {
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
            .push(views::LimiterCallView {
                descriptors: descriptors.to_vec(),
                increments: increments.to_vec(),
            });
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
