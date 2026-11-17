#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use rstest::rstest;

use super::settings::{COST_VARIABLE, cost_per_step_from_text};
use super::support::{capture_logs, cost_text};
use super::{TINYFISH_DEFAULT_STEP_COST, resolve_tinyfish_cost_per_step};

#[rstest]
#[case::unset(None, "0.016")]
#[case::an_empty_value(Some(""), "0.016")]
#[case::whitespace_only(Some("   "), "0.016")]
#[case::a_plain_rate(Some("0.5"), "0.5")]
#[case::surrounding_spaces(Some(" 0.25 "), "0.25")]
#[case::a_scientific_rate(Some("1e-3"), "0.001")]
#[case::a_whole_rate(Some("2"), "2")]
#[case::an_explicit_zero(Some("0"), "0")]
#[case::infinity(Some("inf"), "0.016")]
#[case::nan(Some("NaN"), "0.016")]
#[case::an_underscored_number(Some("1_0"), "0.016")]
fn resolves_the_cost_per_step(#[case] configured: Option<&str>, #[case] expected: &str) {
    let rate = cost_per_step_from_text(configured);
    assert_eq!(cost_text(Some(rate)), Some(expected.to_owned()));
}

#[test]
fn warns_and_keeps_the_default_for_a_value_that_is_not_a_number() {
    let (logs, _guard) = capture_logs();
    let rate = cost_per_step_from_text(Some("not-a-number"));
    assert_eq!(rate, TINYFISH_DEFAULT_STEP_COST);
    assert!(logs.contents().contains("TINYFISH_COST_PER_STEP"));
    assert!(logs.contents().contains("using the default rate 0.016"));
}

#[test]
fn reads_an_empty_value_as_unset() {
    let (logs, _guard) = capture_logs();
    let rate = cost_per_step_from_text(Some(""));
    assert_eq!(rate, TINYFISH_DEFAULT_STEP_COST);
    assert!(!logs.contents().contains("TINYFISH_COST_PER_STEP"));
}

#[test]
fn the_public_rate_follows_the_process_environment() {
    let environment = process_environment();
    assert_eq!(
        resolve_tinyfish_cost_per_step(),
        cost_per_step_from_text(environment.get(COST_VARIABLE).map(String::as_str))
    );
}
