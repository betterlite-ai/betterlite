use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedReserveResult, BatchEnqueuedTokenOverLimit, BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope, ReservationBackend,
};
use betterlite_core::api::batch_limit_counters::BatchLimitCounters;
use betterlite_core::api::batch_limit_counters::input::{
    BatchLimitCounterCall, BatchLimitCountersInput,
};
use betterlite_core::api::batch_limit_counters::ports::{
    BatchCounterLimiter, BatchCounterStash, BatchEnqueuedSource, BatchWindowCache,
};
use betterlite_core::api::batch_limit_counters::records::{
    BatchCounterLimitType, BatchCounterResponse, BatchTpdRefundOperation, WindowStartValue,
};
use betterlite_core::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use betterlite_core::api::batch_limit_descriptors::input::{BatchLimitPrincipal, BatchModelUsage};
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use scripts::{BackendName, CachedWindow, LimiterScript, StoreReply, StoreScript};

mod scripts;
mod views;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    principal: BatchLimitPrincipal,
    clock: f64,
    window_size: i64,
    #[serde(default)]
    cache: BTreeMap<String, Option<CachedWindow>>,
    store: StoreScript,
    limiter: LimiterScript,
    calls: Vec<BatchLimitCounterCall>,
}

#[derive(Clone)]
struct ScriptedStore {
    replies: Arc<Mutex<VecDeque<StoreReply>>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedStore {
    fn new(script: &StoreScript) -> Self {
        Self {
            replies: Arc::new(Mutex::new(script.replies.iter().cloned().collect())),
            calls: Arc::default(),
        }
    }

    fn calls(&self) -> Vec<Value> {
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
    type Error = std::io::Error;

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
struct ScriptedCache {
    values: BTreeMap<String, Option<CachedWindow>>,
    reads: Arc<Mutex<Vec<String>>>,
}

impl ScriptedCache {
    fn new(values: &BTreeMap<String, Option<CachedWindow>>) -> Self {
        Self {
            values: values.clone(),
            reads: Arc::default(),
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BatchWindowCache for ScriptedCache {
    type Error = std::io::Error;

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
struct ScriptedLimiter {
    base: Vec<BatchLimitDescriptor>,
    project: (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>),
    responses: Arc<Mutex<VecDeque<BatchCounterResponse>>>,
    base_calls: Arc<Mutex<usize>>,
    project_calls: Arc<Mutex<usize>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedLimiter {
    fn new(script: &LimiterScript) -> Self {
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

    fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn base_calls(&self) -> usize {
        *self
            .base_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn project_calls(&self) -> usize {
        *self
            .project_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl BatchCounterLimiter for ScriptedLimiter {
    type Error = std::io::Error;

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
            .ok_or_else(|| std::io::Error::other("the scripted limiter has no response left"));
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

#[derive(Default)]
struct RecordingStash {
    reservation: Mutex<Option<BatchEnqueuedTokenReservation>>,
    operations: Mutex<Vec<BatchTpdRefundOperation>>,
}

impl RecordingStash {
    fn reservation(&self) -> Option<BatchEnqueuedTokenReservation> {
        self.reservation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn operations(&self) -> Vec<BatchTpdRefundOperation> {
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BatchCounterStash for RecordingStash {
    fn set_batch_enqueued_reservation(&self, reservation: BatchEnqueuedTokenReservation) {
        *self
            .reservation
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reservation);
    }

    fn set_batch_tpd_refund_ops(&self, operations: Vec<BatchTpdRefundOperation>) {
        *self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = operations;
    }
}

// rate_limiter_utils.resolve_llm_provider_for_rate_limit's native lookup, pinned to the model the cases use
struct FixedProviders;

impl ProviderResolution for FixedProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Ok(ResolvedModel {
            model: model.to_owned(),
            provider: ProviderIdentity::Custom(String::from("openai")),
            api_base: None,
            api_key: None,
        })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let store = ScriptedStore::new(&input.store);
        let limiter = ScriptedLimiter::new(&input.limiter);
        let cache = ScriptedCache::new(&input.cache);
        let stash = Arc::new(RecordingStash::default());
        let clock = input.clock;
        let service = BatchLimitCounters {
            clock: Arc::new(move || clock),
            limiter: limiter.clone(),
            store: store.clone(),
            cache: cache.clone(),
            stash: Arc::clone(&stash),
            resolver: Arc::new(FixedProviders),
            router: None,
            window_size: input.window_size,
            limits: ApiLimits::default(),
        };
        let calls = BatchLimitCountersInput {
            principal: input.principal.clone(),
            calls: input.calls.clone(),
        };
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let result = runtime.block_on(service.run(&calls)).or_value_error()?;
        let answers = result
            .answers
            .iter()
            .map(views::answer_view)
            .collect::<Vec<_>>();
        let held = stash.reservation();
        let operations = stash.operations();
        let effects = json!({
            "store_calls": store.calls(),
            "limiter_calls": limiter.calls(),
            "base_calls": limiter.base_calls(),
            "project_calls": limiter.project_calls(),
            "cache_reads": cache.reads(),
            "reservation": held.as_ref().map(views::reservation_view),
            "refund_ops": operations.iter().map(views::operation_view).collect::<Vec<_>>(),
        });
        Ok(json!({"answers": answers, "effects": effects}))
    }
}
