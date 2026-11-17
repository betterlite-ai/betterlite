#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use rstest::rstest;

use super::settings::cost_per_step_from_environment;
use super::support::{capture_logs, cost_text, environment};
use super::{TINYFISH_DEFAULT_STEP_COST, resolve_tinyfish_cost_per_step};

#[rstest]
#[case::unset(&[], "0.016")]
#[case::an_empty_value(&[("TINYFISH_COST_PER_STEP", "")], "0.016")]
#[case::whitespace_only(&[("TINYFISH_COST_PER_STEP", "   ")], "0.016")]
#[case::another_variable(&[("TINYFISH_AGENT_API_BASE", "0.5")], "0.016")]
#[case::a_plain_rate(&[("TINYFISH_COST_PER_STEP", "0.5")], "0.5")]
#[case::surrounding_spaces(&[("TINYFISH_COST_PER_STEP", " 0.25 ")], "0.25")]
#[case::a_scientific_rate(&[("TINYFISH_COST_PER_STEP", "1e-3")], "0.001")]
#[case::a_whole_rate(&[("TINYFISH_COST_PER_STEP", "2")], "2")]
#[case::an_explicit_zero(&[("TINYFISH_COST_PER_STEP", "0")], "0")]
#[case::infinity(&[("TINYFISH_COST_PER_STEP", "inf")], "0.016")]
#[case::nan(&[("TINYFISH_COST_PER_STEP", "NaN")], "0.016")]
#[case::an_underscored_number(&[("TINYFISH_COST_PER_STEP", "1_0")], "0.016")]
fn resolves_the_cost_per_step(#[case] entries: &[(&str, &str)], #[case] expected: &str) {
    let rate = cost_per_step_from_environment(&environment(entries));
    assert_eq!(cost_text(Some(rate)), Some(expected.to_owned()));
}

#[test]
fn warns_and_keeps_the_default_for_a_value_that_is_not_a_number() {
    let (logs, _guard) = capture_logs();
    let rate =
        cost_per_step_from_environment(&environment(&[("TINYFISH_COST_PER_STEP", "not-a-number")]));
    assert_eq!(rate, TINYFISH_DEFAULT_STEP_COST);
    assert!(logs.contents().contains("TINYFISH_COST_PER_STEP"));
    assert!(logs.contents().contains("using the default rate 0.016"));
}

#[test]
fn reads_an_empty_value_as_unset() {
    let (logs, _guard) = capture_logs();
    let rate = cost_per_step_from_environment(&environment(&[("TINYFISH_COST_PER_STEP", "")]));
    assert_eq!(rate, TINYFISH_DEFAULT_STEP_COST);
    assert!(!logs.contents().contains("TINYFISH_COST_PER_STEP"));
}

#[test]
fn the_public_rate_follows_the_process_environment() {
    assert_eq!(
        resolve_tinyfish_cost_per_step(),
        cost_per_step_from_environment(&process_environment())
    );
}
