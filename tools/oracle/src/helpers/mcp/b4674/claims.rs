use std::collections::HashMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_mcp::gateway_dcr::single_use::{
    ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock, ClaimOutcome, MemoryClaimStore,
    PeekOutcome, SingleUseGuard,
};
use serde_json::{Value, json};

use super::{Action, Authority, BackendKind, ClaimScenario};

const FIXED_NOW: i64 = 1_700_000_000;

struct FixedClock(i64);

impl ClaimClock for FixedClock {
    fn now_seconds(&self) -> i64 {
        self.0
    }
}

struct CountingBackend {
    ttl: i64,
    counts: Mutex<HashMap<String, u64>>,
}

impl CountingBackend {
    fn new(ttl: i64) -> Self {
        Self {
            ttl,
            counts: Mutex::new(HashMap::new()),
        }
    }
}

impl ClaimBackend for CountingBackend {
    fn increment(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        if ttl_seconds != self.ttl {
            return ready(Err(ClaimBackendError));
        }
        let count = {
            let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            let count = counts.entry(key.to_owned()).or_insert(0);
            *count = count.saturating_add(1);
            *count
        };
        ready(Ok(count))
    }

    fn read(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        let count = {
            let counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            counts.get(key).copied()
        };
        ready(Ok(count))
    }
}

struct FixedBackend {
    count: u64,
    ttl: i64,
}

impl ClaimBackend for FixedBackend {
    fn increment(
        &self,
        _key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        if ttl_seconds != self.ttl {
            return ready(Err(ClaimBackendError));
        }
        ready(Ok(self.count))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Ok(Some(self.count)))
    }
}

struct FaultyBackend;

impl ClaimBackend for FaultyBackend {
    fn increment(
        &self,
        _key: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }
}

pub(super) async fn reports(scenarios: &[ClaimScenario]) -> Vec<Value> {
    let mut reports = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let results = match scenario.authority {
            Authority::Local => local_results(scenario).await,
            Authority::Shared => shared_results(scenario).await,
        };
        reports.push(json!({"name": scenario.name, "results": results}));
    }
    reports
}

async fn local_results(scenario: &ClaimScenario) -> Vec<String> {
    let store = MemoryClaimStore::new(Box::new(FixedClock(FIXED_NOW)));
    for key in &scenario.seed_keys {
        let _seeded = store.increment(key, scenario.ttl);
    }
    let authority = ClaimAuthority::Local(store);
    guard_results::<FaultyBackend>(authority, scenario).await
}

async fn shared_results(scenario: &ClaimScenario) -> Vec<String> {
    let ttl = scenario.ttl;
    match scenario.backend {
        BackendKind::Counting => {
            let authority = ClaimAuthority::Shared(CountingBackend::new(ttl));
            guard_results(authority, scenario).await
        }
        BackendKind::FixedZero => {
            let authority = ClaimAuthority::Shared(FixedBackend { count: 0, ttl });
            guard_results(authority, scenario).await
        }
        BackendKind::FixedOne => {
            let authority = ClaimAuthority::Shared(FixedBackend { count: 1, ttl });
            guard_results(authority, scenario).await
        }
        BackendKind::FixedTwo => {
            let authority = ClaimAuthority::Shared(FixedBackend { count: 2, ttl });
            guard_results(authority, scenario).await
        }
        BackendKind::Faulty => guard_results(ClaimAuthority::Shared(FaultyBackend), scenario).await,
    }
}

async fn guard_results<S: ClaimBackend>(
    authority: ClaimAuthority<S>,
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
