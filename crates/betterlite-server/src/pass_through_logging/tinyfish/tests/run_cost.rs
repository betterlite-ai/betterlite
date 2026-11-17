#![cfg(test)]

use rstest::rstest;

use super::records::run_cost;
use super::support::{decimal, run};

#[rstest]
#[case::one_step(
    r#"{"status": "COMPLETED", "num_of_steps": 1}"#,
    "0.016",
    Some("0.016")
)]
#[case::three_steps(
    r#"{"status": "COMPLETED", "num_of_steps": 3}"#,
    "0.016",
    Some("0.048")
)]
#[case::zero_steps(r#"{"status": "COMPLETED", "num_of_steps": 0}"#, "0.016", Some("0"))]
#[case::ten_steps(
    r#"{"status": "COMPLETED", "num_of_steps": 10}"#,
    "0.016",
    Some("0.16")
)]
#[case::hundred_steps(
    r#"{"status": "COMPLETED", "num_of_steps": 100}"#,
    "0.016",
    Some("1.6")
)]
#[case::negative_steps(
    r#"{"status": "COMPLETED", "num_of_steps": -2}"#,
    "0.016",
    Some("-0.032")
)]
#[case::another_rate(r#"{"status": "COMPLETED", "num_of_steps": 3}"#, "0.5", Some("1.5"))]
#[case::a_rate_with_a_float_product(
    r#"{"status": "COMPLETED", "num_of_steps": 3}"#,
    "0.07",
    Some("0.21")
)]
#[case::zero_rate(r#"{"status": "COMPLETED", "num_of_steps": 3}"#, "0", Some("0"))]
#[case::failed_run(r#"{"status": "FAILED", "num_of_steps": 3}"#, "0.016", None)]
#[case::cancelled_run(r#"{"status": "CANCELLED", "num_of_steps": 3}"#, "0.016", None)]
#[case::pending_run(r#"{"status": "PENDING", "num_of_steps": 3}"#, "0.016", None)]
#[case::lowercase_status(r#"{"status": "completed", "num_of_steps": 3}"#, "0.016", None)]
#[case::null_status(r#"{"status": null, "num_of_steps": 3}"#, "0.016", None)]
#[case::absent_status(r#"{"num_of_steps": 3}"#, "0.016", None)]
#[case::null_steps(r#"{"status": "COMPLETED", "num_of_steps": null}"#, "0.016", None)]
#[case::absent_steps(r#"{"status": "COMPLETED"}"#, "0.016", None)]
fn prices_only_a_completed_run(
    #[case] text: &str,
    #[case] rate: &str,
    #[case] expected: Option<&str>,
) {
    let submitted = run(text);
    let cost = run_cost(Some(&submitted), decimal(rate));
    assert_eq!(
        cost.map(|value| value.normalize().to_string()),
        expected.map(ToOwned::to_owned)
    );
}

#[test]
fn a_missing_run_costs_nothing() {
    assert_eq!(run_cost(None, decimal("0.016")), None);
}
