use std::collections::HashMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_mcp::gateway_dcr::single_use::{
    ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock, ClaimOutcome, MemoryClaimStore,
    PeekOutcome, SingleUseGuard,
};
use serde_json::{Value, json};

use super::{Action, Authority, BackendKind, ClaimScenario};

struct FrozenClock;
impl ClaimClock for FrozenClock {
    fn now_seconds(&self) -> i64 {
        0
    }
}

struct Backend {
    ttl: i64,
    fixed: Option<u64>,
    faulty: bool,
    counts: Mutex<HashMap<String, u64>>,
}
impl Backend {
    fn new(kind: BackendKind, ttl: i64) -> Self {
        let (fixed, faulty) = match kind {
            BackendKind::Counting => (None, false),
            BackendKind::FixedZero => (Some(0), false),
            BackendKind::FixedOne => (Some(1), false),
            BackendKind::FixedTwo => (Some(2), false),
            BackendKind::Faulty => (None, true),
        };
        Self {
            ttl,
            fixed,
            faulty,
            counts: Mutex::new(HashMap::new()),
        }
    }
}
impl ClaimBackend for Backend {
    fn increment(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        if self.faulty || ttl_seconds != self.ttl {
            return ready(Err(ClaimBackendError));
        }
        let Some(fixed) = self.fixed else {
            let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            let count = counts.entry(key.to_owned()).or_insert(0);
            *count = count.saturating_add(1);
            let total = *count;
            drop(counts);
            return ready(Ok(total));
        };
        ready(Ok(fixed))
    }

    fn read(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        match (self.faulty, self.fixed) {
            (true, _fixed) => ready(Err(ClaimBackendError)),
            (false, Some(fixed)) => ready(Ok(Some(fixed))),
            (false, None) => {
                let counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
                ready(Ok(counts.get(key).copied()))
            }
        }
    }
}

pub(super) async fn reports(scenarios: &[ClaimScenario]) -> Vec<Value> {
    let mut reports = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let results = scenario_results(scenario).await;
        reports.push(json!({"name": scenario.name, "results": results}));
    }
    reports
}

async fn scenario_results(scenario: &ClaimScenario) -> Vec<String> {
    let authority = match scenario.authority {
        Authority::Local => ClaimAuthority::Local(seeded_store(scenario)),
        Authority::Shared => ClaimAuthority::Shared(Backend::new(scenario.backend, scenario.ttl)),
    };
    guard_results(authority, scenario).await
}

fn seeded_store(scenario: &ClaimScenario) -> MemoryClaimStore {
    let store = MemoryClaimStore {
        clock: Box::new(FrozenClock),
        entries: Mutex::new(HashMap::new()),
    };
    for key in &scenario.seed_keys {
        let _seeded = store.increment(key, scenario.ttl);
    }
    store
}

async fn guard_results(
    authority: ClaimAuthority<Backend>,
    scenario: &ClaimScenario,
) -> Vec<String> {
    let guard = SingleUseGuard::from(authority);
    let mut results = Vec::with_capacity(scenario.steps.len());
    for step in &scenario.steps {
        let outcome = match step.action {
            Action::Claim => match guard.claim(&step.key, scenario.ttl).await {
                ClaimOutcome::First => "first",
                ClaimOutcome::Replayed => "replayed",
                ClaimOutcome::Unavailable => "unavailable",
            },
            Action::Peek => match guard.peek(&step.key).await {
                PeekOutcome::Unclaimed => "unclaimed",
                PeekOutcome::Claimed => "claimed",
                PeekOutcome::Unavailable => "unavailable",
            },
        };
        results.push(outcome.to_owned());
    }
    results
}
