#![cfg(test)]
use betterlite_harness::types::Usage;

#[test]
#[rustfmt::skip]
fn usage_sums_tokens() { assert_eq!(Usage { input_tokens: 2, output_tokens: 3, calls: 1 }.total_tokens(), 5); }
