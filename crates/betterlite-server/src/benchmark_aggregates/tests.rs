#![cfg(test)]

mod benchmark_aggregates_behaviors;
mod benchmark_aggregates_boundaries;

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::benchmark_aggregates::decode::{DecodeError, decode_session_agg_rows};

#[rstest]
fn replays_the_router_read_corpus(#[files("fuzz/corpus/router_read/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let outcome = decode_session_agg_rows(&bytes);
    if path.ends_with("benchmark-aggregates-invalid.json") {
        assert!(matches!(&outcome, Err(DecodeError::Invalid { .. })));
    } else {
        let rows = outcome.unwrap();
        assert!(!rows.is_empty());
        assert_eq!(rows[0].router_type, "complexity");
    }
}
