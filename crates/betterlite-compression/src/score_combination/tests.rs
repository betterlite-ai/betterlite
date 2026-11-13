#![cfg(test)]

use super::*;

fn close(actual: f64, expected: f64) -> bool {
    expected.mul_add(-1.0, actual).abs() < 1e-9
}

#[test]
fn default_weight_matches_litellm() {
    assert_eq!(DEFAULT_BM25_WEIGHT.to_bits(), 0.4_f64.to_bits());
}

#[test]
fn normalizes_each_score_list_independently() {
    let combined = combine_scores(&[0.0, 10.0], &[10.0, 0.0], DEFAULT_BM25_WEIGHT);
    assert!(close(combined[0], 0.6));
    assert!(close(combined[1], 0.4));
}

#[test]
fn weights_bm25_at_four_tenths_and_embeddings_at_six_tenths() {
    let combined = combine_scores(&[10.0, 0.0], &[0.0, 10.0], DEFAULT_BM25_WEIGHT);
    assert!(close(combined[0], 0.4));
    assert!(close(combined[1], 0.6));
}

#[test]
fn zero_range_scores_become_zero() {
    assert_eq!(
        combine_scores(&[5.0, 5.0], &[7.0, 7.0], DEFAULT_BM25_WEIGHT),
        vec![0.0, 0.0]
    );
    assert_eq!(
        combine_scores(&[5.0], &[7.0], DEFAULT_BM25_WEIGHT),
        vec![0.0]
    );
}

#[test]
fn empty_lists_give_no_scores() {
    assert!(combine_scores(&[], &[], DEFAULT_BM25_WEIGHT).is_empty());
}

#[test]
fn normalizes_negative_scores() {
    let combined = combine_scores(&[-10.0, 0.0], &[0.0, 10.0], DEFAULT_BM25_WEIGHT);
    assert!(close(combined[0], 0.0));
    assert!(close(combined[1], 1.0));
}

#[test]
fn uses_the_supplied_weight() {
    let combined = combine_scores(&[10.0, 0.0], &[0.0, 10.0], 0.25);
    assert!(close(combined[0], 0.25));
    assert!(close(combined[1], 0.75));
}

#[test]
fn a_weight_of_one_keeps_only_bm25() {
    let combined = combine_scores(&[10.0, 0.0], &[0.0, 10.0], 1.0);
    assert!(close(combined[0], 1.0));
    assert!(close(combined[1], 0.0));
}

#[test]
fn pairs_scores_up_to_the_shorter_list() {
    let combined = combine_scores(&[0.0, 10.0, 20.0], &[0.0, 10.0], DEFAULT_BM25_WEIGHT);
    assert_eq!(combined.len(), 2);
    assert!(close(combined[0], 0.0));
    assert!(close(combined[1], 0.8));
}
