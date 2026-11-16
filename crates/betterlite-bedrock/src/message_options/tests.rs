#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::cache_ttl_model_policy(cases::cache_ttl_model_policy)]
#[case::clear_thinking_budget(cases::clear_thinking_budget)]
#[case::adaptive_clear_thinking(cases::adaptive_clear_thinking)]
#[case::effort_clamp(cases::effort_clamp)]
#[case::extended_thinking_support(cases::extended_thinking_support)]
#[case::clear_thinking_injection_limit(cases::clear_thinking_injection_limit)]
fn message_options_cases(#[case] case: fn()) {
    case();
}
