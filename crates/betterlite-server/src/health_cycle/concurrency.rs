use futures_util::future::join_all;
use futures_util::stream::{FuturesUnordered, StreamExt as _};

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

// _run_health_checks_with_bounded_concurrency: fill the queue, then replace each finished probe until the list runs out
async fn bounded(
    prepared: &[PreparedProbe],
    provider: &dyn ProviderHealth,
    limit: usize,
) -> Vec<ProbeOutcome> {
    let mut slots: Vec<Option<ProbeOutcome>> = vec![None; prepared.len()];
    let mut pending = FuturesUnordered::new();
    let mut models = prepared.iter().enumerate();
    loop {
        while pending.len() < limit {
            let Some((index, probe)) = models.next() else {
                break;
            };
            pending.push(async move { (index, execute_probe(provider, probe).await) });
        }
        let Some((index, outcome)) = pending.next().await else {
            break;
        };
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(outcome);
        }
    }
    slots.into_iter().flatten().collect()
}

// asyncio.gather: every probe starts at once, and return_exceptions=True never lets one failure drop a sibling
#[must_use]
pub async fn run_probes(
    prepared: &[PreparedProbe],
    provider: &dyn ProviderHealth,
    max_concurrency: Option<i64>,
) -> (Vec<ProbeOutcome>, DispatchMode, usize) {
    match max_concurrency
        .filter(|limit| *limit > 0)
        .and_then(|limit| usize::try_from(limit).ok())
    {
        Some(limit) => {
            let outcomes = bounded(prepared, provider, limit).await;
            (outcomes, DispatchMode::Bounded, limit.min(prepared.len()))
        }
        None => (
            join_all(prepared.iter().map(|probe| execute_probe(provider, probe))).await,
            DispatchMode::Unbounded,
            prepared.len(),
        ),
    }
}
