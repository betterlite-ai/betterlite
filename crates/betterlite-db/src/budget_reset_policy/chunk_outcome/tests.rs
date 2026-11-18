#![cfg(test)]

use super::{ChunkOutcome, NO_PROGRESS};

#[test]
fn chunk_outcome_no_progress_reports_nothing_fetched_or_advanced() {
    assert_eq!(NO_PROGRESS.fetched, 0);
    assert_eq!(NO_PROGRESS.advanced, 0);
    assert_eq!(NO_PROGRESS, ChunkOutcome::default());
}

#[test]
fn chunk_outcome_keeps_the_counts_of_one_chunk() {
    let outcome = ChunkOutcome {
        fetched: 7,
        advanced: 3,
    };
    assert_eq!(outcome.fetched, 7);
    assert_eq!(outcome.advanced, 3);
    assert_ne!(outcome, NO_PROGRESS);
}
