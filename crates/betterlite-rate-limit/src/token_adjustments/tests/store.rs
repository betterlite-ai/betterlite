#![cfg(test)]

use crate::token_adjustments::store::{TokenAdjustmentError, token_float};

#[test]
fn token_float_keeps_zero() {
    assert_eq!(token_float(0).expect("zero"), 0.0);
}

#[test]
fn token_float_keeps_the_largest_exact_count() {
    assert_eq!(
        token_float(9_007_199_254_740_992).expect("the largest exact count"),
        9_007_199_254_740_992.0
    );
}

#[test]
fn token_float_rejects_the_next_count() {
    let result = token_float(9_007_199_254_740_993);
    assert!(matches!(
        result,
        Err(TokenAdjustmentError::TokenAmount {
            value: 9_007_199_254_740_993
        })
    ));
}

#[test]
fn token_float_keeps_a_negative_count_in_range() {
    assert_eq!(token_float(-5).expect("a small refund"), -5.0);
}

#[test]
fn token_float_rejects_a_negative_count_past_the_boundary() {
    let result = token_float(-9_007_199_254_740_993);
    assert!(matches!(
        result,
        Err(TokenAdjustmentError::TokenAmount {
            value: -9_007_199_254_740_993
        })
    ));
}
