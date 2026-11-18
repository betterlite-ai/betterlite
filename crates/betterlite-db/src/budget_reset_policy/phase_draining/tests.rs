#![cfg(test)]

use super::{
    BATCH_SIZE_ENV, DEFAULT_BATCH_SIZE, DEFAULT_MAX_CHUNKS, MAX_CHUNKS_ENV, ResetLimits,
    ResetLimitsError, phase_is_drained, run_phase_in_chunks,
};
use crate::budget_reset_policy::chunk_outcome::{ChunkOutcome, NO_PROGRESS};
use betterlite_env::EnvError;
use rstest::rstest;
use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn environment<const N: usize>(entries: [(&str, &str); N]) -> BTreeMap<String, String> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn phase_draining_defaults_to_500_rows_and_100_chunks() {
    assert_eq!(
        ResetLimits::default(),
        ResetLimits {
            batch_size: DEFAULT_BATCH_SIZE,
            max_chunks_per_run: DEFAULT_MAX_CHUNKS,
        }
    );
    assert_eq!(DEFAULT_BATCH_SIZE, 500);
    assert_eq!(DEFAULT_MAX_CHUNKS, 100);
}

#[test]
fn phase_draining_reads_both_environment_limits() {
    let configured = environment([(BATCH_SIZE_ENV, "50"), (MAX_CHUNKS_ENV, "7")]);
    assert_eq!(
        ResetLimits::from_environment(&configured).unwrap(),
        ResetLimits {
            batch_size: 50,
            max_chunks_per_run: 7,
        }
    );
}

#[test]
fn phase_draining_defaults_a_missing_limit() {
    let configured = environment([]);
    assert_eq!(
        ResetLimits::from_environment(&configured).unwrap(),
        ResetLimits::default()
    );
}

#[test]
fn phase_draining_defaults_an_empty_limit() {
    let configured = environment([(BATCH_SIZE_ENV, "  "), (MAX_CHUNKS_ENV, "")]);
    assert_eq!(
        ResetLimits::from_environment(&configured).unwrap(),
        ResetLimits::default()
    );
}

#[test]
fn phase_draining_clamps_a_zero_limit_to_one() {
    let configured = environment([(BATCH_SIZE_ENV, "0"), (MAX_CHUNKS_ENV, "0")]);
    assert_eq!(
        ResetLimits::from_environment(&configured).unwrap(),
        ResetLimits {
            batch_size: 1,
            max_chunks_per_run: 1,
        }
    );
}

#[test]
fn phase_draining_clamps_a_negative_limit_to_one() {
    let configured = environment([(BATCH_SIZE_ENV, "-4"), (MAX_CHUNKS_ENV, "-1")]);
    assert_eq!(
        ResetLimits::from_environment(&configured).unwrap(),
        ResetLimits {
            batch_size: 1,
            max_chunks_per_run: 1,
        }
    );
}

#[test]
fn phase_draining_rejects_a_non_numeric_limit() {
    let configured = environment([(BATCH_SIZE_ENV, "many"), (MAX_CHUNKS_ENV, "7")]);
    let error = ResetLimits::from_environment(&configured).unwrap_err();
    assert!(matches!(
        error,
        ResetLimitsError::Environment(EnvError::Number { ref name }) if name == BATCH_SIZE_ENV
    ));
}

#[rstest]
#[case::short_chunk(ChunkOutcome { fetched: 499, advanced: 10 }, 500, true)]
#[case::one_row_short(ChunkOutcome { fetched: 3, advanced: 1 }, 4, true)]
#[case::full_chunk_advancing(ChunkOutcome { fetched: 500, advanced: 10 }, 500, false)]
#[case::full_chunk_at_the_batch_size(ChunkOutcome { fetched: 4, advanced: 1 }, 4, false)]
#[case::full_chunk_without_advances(ChunkOutcome { fetched: 500, advanced: 0 }, 500, true)]
#[case::empty_chunk(NO_PROGRESS, 500, true)]
fn phase_draining_is_drained(
    #[case] outcome: ChunkOutcome,
    #[case] batch_size: usize,
    #[case] drained: bool,
) {
    assert_eq!(phase_is_drained(outcome, batch_size), drained);
}

#[derive(Clone, Default)]
struct Runs(Arc<AtomicUsize>);

impl Runs {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }

    fn record(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn scripted(outcomes: Vec<ChunkOutcome>, runs: &Runs) -> impl FnMut() -> Ready<ChunkOutcome> {
    let recorded = runs.clone();
    let mut remaining = outcomes.into_iter();
    move || {
        recorded.record();
        ready(remaining.next().unwrap_or(NO_PROGRESS))
    }
}

#[tokio::test]
async fn phase_draining_stops_after_a_short_chunk() {
    let runs = Runs::default();
    let process = scripted(
        vec![ChunkOutcome {
            fetched: 1,
            advanced: 1,
        }],
        &runs,
    );
    run_phase_in_chunks(
        ResetLimits {
            batch_size: 2,
            max_chunks_per_run: 10,
        },
        process,
    )
    .await;
    assert_eq!(runs.count(), 1);
}

#[tokio::test]
async fn phase_draining_stops_when_a_full_chunk_advances_nothing() {
    let runs = Runs::default();
    let process = scripted(
        vec![ChunkOutcome {
            fetched: 2,
            advanced: 0,
        }],
        &runs,
    );
    run_phase_in_chunks(
        ResetLimits {
            batch_size: 2,
            max_chunks_per_run: 10,
        },
        process,
    )
    .await;
    assert_eq!(runs.count(), 1);
}

#[tokio::test]
async fn phase_draining_runs_until_a_chunk_drains() {
    let runs = Runs::default();
    let process = scripted(
        vec![
            ChunkOutcome {
                fetched: 2,
                advanced: 1,
            },
            ChunkOutcome {
                fetched: 1,
                advanced: 1,
            },
            ChunkOutcome {
                fetched: 2,
                advanced: 2,
            },
        ],
        &runs,
    );
    run_phase_in_chunks(
        ResetLimits {
            batch_size: 2,
            max_chunks_per_run: 10,
        },
        process,
    )
    .await;
    assert_eq!(runs.count(), 2);
}

#[tokio::test]
async fn phase_draining_caps_a_run_at_the_configured_chunks() {
    let runs = Runs::default();
    let recorded = runs.clone();
    run_phase_in_chunks(
        ResetLimits {
            batch_size: 2,
            max_chunks_per_run: 3,
        },
        move || {
            recorded.record();
            ready(ChunkOutcome {
                fetched: 2,
                advanced: 1,
            })
        },
    )
    .await;
    assert_eq!(runs.count(), 3);
}

#[tokio::test]
async fn phase_draining_default_cap_is_a_hundred_chunks() {
    let runs = Runs::default();
    let recorded = runs.clone();
    run_phase_in_chunks(ResetLimits::default(), move || {
        recorded.record();
        ready(ChunkOutcome {
            fetched: DEFAULT_BATCH_SIZE,
            advanced: 1,
        })
    })
    .await;
    assert_eq!(runs.count(), DEFAULT_MAX_CHUNKS);
}
