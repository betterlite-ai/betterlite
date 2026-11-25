use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_core::api::batch_limit_counters::BatchLimitCounters;
use betterlite_core::api::batch_limit_counters::input::{
    BatchLimitCounterCall, BatchLimitCountersInput,
};
use betterlite_core::api::batch_limit_counters::ports::BatchCounterStash;
use betterlite_core::api::batch_limit_counters::records::BatchTpdRefundOperation;
use betterlite_core::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use serde::Deserialize;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fakes::{ScriptedCache, ScriptedLimiter, ScriptedStore};
use scripts::{CachedWindow, LimiterScript, StoreScript};

mod fakes;
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
        let stash_handle: Arc<dyn BatchCounterStash> = Arc::<RecordingStash>::clone(&stash);
        let clock = input.clock;
        let service = BatchLimitCounters {
            clock: Arc::new(move || clock),
            limiter: limiter.clone(),
            store: store.clone(),
            cache: cache.clone(),
            stash: stash_handle,
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
        let output = views::HelperOutput {
            answers: result.answers.iter().map(views::AnswerView::from).collect(),
            effects: scripted_effects(&store, &limiter, &cache, &stash),
        };
        serde_json::to_value(output).or_value_error()
    }
}

fn scripted_effects(
    store: &ScriptedStore,
    limiter: &ScriptedLimiter,
    cache: &ScriptedCache,
    stash: &RecordingStash,
) -> views::EffectsView {
    views::EffectsView {
        store_calls: store.calls(),
        limiter_calls: limiter.calls(),
        base_calls: limiter.base_calls(),
        project_calls: limiter.project_calls(),
        cache_reads: cache.reads(),
        reservation: stash
            .reservation()
            .as_ref()
            .map(views::ReservationView::from),
        refund_ops: stash.operations(),
    }
}
