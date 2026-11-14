#![cfg(test)]

use rstest::rstest;

use super::support::{call_info, decimal};
use super::thresholds::get_percent_of_max_budget_left;

#[rstest]
#[case("100", None, "0")]
#[case("100", Some("0"), "0")]
#[case("5", Some("-10"), "0")]
#[case("0", Some("100"), "1")]
#[case("25", Some("100"), "0.75")]
#[case("100", Some("100"), "0")]
#[case("150", Some("100"), "-0.5")]
#[case("95", Some("100"), "0.05")]
fn the_percent_left_is_the_remaining_share_of_the_budget(
    #[case] spend: &str,
    #[case] max_budget: Option<&str>,
    #[case] expected: &str,
) {
    let mut info = call_info(decimal(spend));
    info.max_budget = max_budget.map(decimal);

    assert_eq!(get_percent_of_max_budget_left(&info), decimal(expected));
}
