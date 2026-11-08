use rstest::rstest;

use super::support;
use crate::fetch_policy::integrity::{IntegrityError, validate_model_cost_map};

#[derive(Debug, PartialEq)]
enum Validated {
    Accepted,
    Empty,
    TooFew { found: usize, floor: usize },
    Shrunk { found: usize, previous: usize },
}

#[rstest]
#[case::empty(&[], 0, 0, "0.5", Validated::Empty)]
#[case::minimum_met(&["model"], 0, 1, "0.5", Validated::Accepted)]
#[case::minimum_boundary(&["a", "b"], 0, 2, "0.5", Validated::Accepted)]
#[case::below_minimum(
    &["a", "b"],
    0,
    3,
    "0.5",
    Validated::TooFew { found: 2, floor: 3 }
)]
#[case::minimum_precedes_shrink(
    &["a", "b", "c"],
    1000,
    50,
    "0.5",
    Validated::TooFew { found: 3, floor: 50 }
)]
#[case::shrink_boundary(&["a", "b"], 4, 0, "0.5", Validated::Accepted)]
#[case::below_shrink_boundary(
    &["a", "b", "c"],
    8,
    0,
    "0.5",
    Validated::Shrunk { found: 3, previous: 8 }
)]
#[case::zero_backup_skips_shrink(&["a"], 0, 0, "0.5", Validated::Accepted)]
#[case::zero_ratio_accepts(&["a"], 1000, 0, "0", Validated::Accepted)]
#[case::overflowing_threshold_shrinks(
    &["a"],
    usize::MAX,
    0,
    "79228162514264337593543950335",
    Validated::Shrunk { found: 1, previous: usize::MAX }
)]
fn catalog_fetch_policy_behavior_tests_behavior(
    #[case] names: &[&str],
    #[case] backup: usize,
    #[case] minimum: usize,
    #[case] ratio: &str,
    #[case] expected: Validated,
) {
    let result = validate_model_cost_map(
        &support::models(names),
        backup,
        support::policy(minimum, ratio),
    );
    let message = format!("{result:?}");
    match expected {
        Validated::Accepted => assert!(result.is_ok(), "{message}"),
        Validated::Empty => {
            assert!(matches!(result, Err(IntegrityError::Empty)), "{message}");
        }
        Validated::TooFew { found, floor } => assert!(
            matches!(
                result,
                Err(IntegrityError::TooFewModels {
                    count: actual,
                    minimum: actual_floor,
                }) if actual == found && actual_floor == floor
            ),
            "{message}"
        ),
        Validated::Shrunk { found, previous } => assert!(
            matches!(
                result,
                Err(IntegrityError::Shrunk {
                    count: actual,
                    backup: actual_previous,
                }) if actual == found && actual_previous == previous
            ),
            "{message}"
        ),
    }
}
