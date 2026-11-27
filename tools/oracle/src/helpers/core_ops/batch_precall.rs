use std::sync::{Arc, Mutex};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, ReservationBackend,
};
use betterlite_core::api::batch_limit_counters::records::{
    BatchFileUsage, BatchTpdRefundOperation,
};
use betterlite_core::api::batch_precall::BatchPrecall;
use betterlite_core::api::batch_precall::input::{BatchPrecallCall, BatchPrecallInput};
use betterlite_error::rate_limit::{ProxyRateLimitError, ProxyRateLimitErrorInit};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use report::report;

mod ports;
mod report;

// One case: the production call and every dependency it reaches, scripted
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) call: BatchPrecallCall,
    pub(crate) stages: Stages,
    pub(crate) stash: Stash,
    pub(crate) memory: Memory,
    pub(crate) remote: Remote,
    pub(crate) verbose_stdout: bool,
}

// B1044's skip decision and B1046's accounting plus B1045's reserve or charge, answered from the script
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Stages {
    pub(crate) skip: bool,
    pub(crate) usage: BatchFileUsage,
    pub(crate) reject: bool,
    pub(crate) rejection: String,
    pub(crate) backend: Option<ReservationBackend>,
    pub(crate) owner: String,
}

// B1045's request stash as the pre-call hook leaves it, and the refund store's failure script
#[derive(Debug, Deserialize)]
pub(crate) struct Stash {
    pub(crate) reservation: Option<Reservation>,
    pub(crate) operations: Vec<BatchTpdRefundOperation>,
    pub(crate) refund_failure: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reservation {
    pub(crate) tokens: i64,
    pub(crate) scopes: Vec<BatchEnqueuedTokenScope>,
    pub(crate) backend: Option<ReservationBackend>,
    pub(crate) owner: String,
}

// the in-memory tier's entries and the failures its methods raise
#[derive(Debug, Deserialize)]
pub(crate) struct Memory {
    pub(crate) entries: IndexMap<String, Json>,
    pub(crate) faults: Vec<Fault>,
}

// the backing store the retrieve warm-up scans and reads; the source's litellm.cache is always a RedisCache here
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Remote {
    pub(crate) scan_keys: Vec<String>,
    pub(crate) batch_entries: IndexMap<String, Json>,
    pub(crate) get_value: Option<Json>,
    pub(crate) faults: Vec<Fault>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Fault {
    pub(crate) function: Function,
    pub(crate) message: String,
    pub(crate) status: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Scan,
    BatchGet,
    Get,
    Set,
    SetMany,
}

// every dependency the two hooks reach, as one scripted object so a case's faults and effects share one order
pub(crate) struct Scripted {
    stages: Stages,
    rejection: Box<ProxyRateLimitError>,
    refund_failure: Option<String>,
    memory_faults: Vec<Fault>,
    remote: Remote,
    verbose_stdout: bool,
    state: Mutex<State>,
}

// the mutable half: the stash the hooks hand each other, the in-memory tier, and the effects in call order
pub(crate) struct State {
    pub(super) held: Option<BatchEnqueuedTokenReservation>,
    pub(super) operations: Vec<BatchTpdRefundOperation>,
    pub(super) memory: IndexMap<String, Json>,
    pub(super) effects: Vec<&'static str>,
}

impl Scripted {
    fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let rejection =
            ProxyRateLimitError::new(rejection_init(&input.stages.rejection)).or_value_error()?;
        Ok(Self {
            stages: input.stages.clone(),
            rejection: Box::new(rejection),
            refund_failure: input.stash.refund_failure.clone(),
            memory_faults: input.memory.faults.clone(),
            remote: input.remote.clone(),
            verbose_stdout: input.verbose_stdout,
            state: Mutex::new(State {
                held: input.stash.reservation.as_ref().map(held),
                operations: input.stash.operations.clone(),
                memory: input.memory.entries.clone(),
                effects: Vec::new(),
            }),
        })
    }

    fn service(
        &self,
    ) -> BatchPrecall<'_, Scripted, Scripted, Scripted, Scripted, Scripted, Scripted> {
        let clock = 0.0_f64;
        BatchPrecall {
            stages: self,
            stash: self,
            refunds: self,
            applier: self,
            memory: self,
            remote: Some(self),
            verbose_stdout: self.verbose_stdout,
            clock: Arc::new(move || clock),
            limits: ApiLimits::default(),
        }
    }
}

fn held(reservation: &Reservation) -> BatchEnqueuedTokenReservation {
    BatchEnqueuedTokenReservation {
        tokens: reservation.tokens,
        scopes: reservation.scopes.clone(),
        backend: reservation.backend.unwrap_or(ReservationBackend::Memory),
        owner: reservation.owner.clone(),
        reserved_at_monotonic: 0.0_f64,
    }
}

fn rejection_init(detail: &str) -> ProxyRateLimitErrorInit {
    ProxyRateLimitErrorInit {
        detail: json!(detail),
        headers: None,
        category: None,
        rate_limit_type: None,
        model: None,
        llm_provider: None,
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let scripted = Scripted::new(input)?;
        let plan = BatchPrecallInput {
            call: input.call.clone(),
        };
        let runtime = Builder::new_current_thread().build()?;
        let result = runtime
            .block_on(scripted.service().run(&plan))
            .or_value_error()?;
        Ok(report(&scripted, &result))
    }
}
