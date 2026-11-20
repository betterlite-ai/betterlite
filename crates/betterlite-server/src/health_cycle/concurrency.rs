use futures_util::future::join_all;
use futures_util::stream::{self, StreamExt};

use crate::health_cycle::ports::ProviderHealth;
use crate::health_cycle::probe::{PreparedProbe, ProbeOutcome, execute_probe};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchMode {
    Bounded,
    Unbounded,
}

impl DispatchMode {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bounded => "bounded",
            Self::Unbounded => "unbounded",
        }
    }
}

// max_concurrency bounds the run only when it is a positive integer; absent, zero and negative all mean gather everything
#[must_use]
pub fn bounded_limit(max_concurrency: Option<i64>) -> Option<usize> {
    let limit = max_concurrency.filter(|limit| *limit > 0)?;
    usize::try_from(limit).ok()
}

// Probe slot order follows the input list, whatever order the providers answer in
async fn bounded(
    prepared: &[PreparedProbe],
    provider: &dyn ProviderHealth,
    limit: usize,
) -> Vec<ProbeOutcome> {
    let mut slots: Vec<Option<ProbeOutcome>> = vec![None; prepared.len()];
    let mut running = stream::iter(prepared.iter().enumerate())
        .map(|(index, probe)| async move { (index, execute_probe(provider, probe).await) })
        .buffer_unordered(limit);
    while let Some((index, outcome)) = running.next().await {
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(outcome);
        }
    }
    slots.into_iter().flatten().collect()
}

// asyncio.gather: every probe starts at once, and return_exceptions=True never lets one failure drop a sibling
async fn unbounded(prepared: &[PreparedProbe], provider: &dyn ProviderHealth) -> Vec<ProbeOutcome> {
    join_all(prepared.iter().map(|probe| execute_probe(provider, probe))).await
}

// The peak in-flight count is how many probes the scheduler holds open at once, which the initial fill already reaches
#[must_use]
pub async fn run_probes(
    prepared: &[PreparedProbe],
    provider: &dyn ProviderHealth,
    max_concurrency: Option<i64>,
) -> (Vec<ProbeOutcome>, DispatchMode, usize) {
    match bounded_limit(max_concurrency) {
        Some(limit) => {
            let outcomes = bounded(prepared, provider, limit).await;
            (outcomes, DispatchMode::Bounded, limit.min(prepared.len()))
        }
        None => (
            unbounded(prepared, provider).await,
            DispatchMode::Unbounded,
            prepared.len(),
        ),
    }
}
