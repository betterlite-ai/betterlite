#![cfg(test)]

use betterlite_anthropic::usage::ReasoningTokenCounter as _;

use super::ZeroReasoning;

#[test]
fn zero_reasoning_counts_no_tokens() {
    assert_eq!(
        ZeroReasoning
            .count_reasoning_tokens("thinking")
            .expect("the counter cannot fail"),
        0
    );
}
