use axum::http::StatusCode;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::key_delegation::budget::check_budget_ceilings;
use crate::key_delegation::caller::Caller;
use crate::key_delegation::error::{DelegationDetail, DelegationError};
use crate::management_guards::amounts::Amount;

use super::{admin, cli_session, ui_session_team_key, user};

fn amount(text: &str) -> Amount {
    Amount::Finite(text.parse::<Decimal>().expect("a decimal amount"))
}

#[test]
fn budget_empty_skips() {
    let empty: [Amount; 0] = [];
    assert_eq!(
        check_budget_ceilings(None, Some(amount("0")), user(), false),
        Ok(())
    );
    assert_eq!(
        check_budget_ceilings(Some(empty.as_slice()), Some(amount("0")), user(), false),
        Ok(())
    );
    assert_eq!(
        check_budget_ceilings(Some(empty.as_slice()), None, admin(), false),
        Ok(())
    );
    // the skip runs before the CLI session's team requirement
    assert_eq!(
        check_budget_ceilings(None, None, cli_session(), false),
        Ok(())
    );
}

#[rstest]
#[case::nan_admin(Amount::Nan, "nan", admin())]
#[case::nan_ui(Amount::Nan, "nan", ui_session_team_key())]
#[case::nan_user(Amount::Nan, "nan", user())]
#[case::nan_cli(Amount::Nan, "nan", cli_session())]
#[case::inf_admin(Amount::PositiveInfinity, "inf", admin())]
#[case::inf_ui(Amount::PositiveInfinity, "inf", ui_session_team_key())]
#[case::inf_user(Amount::PositiveInfinity, "inf", user())]
#[case::inf_cli(Amount::PositiveInfinity, "inf", cli_session())]
#[case::neg_inf_admin(Amount::NegativeInfinity, "-inf", admin())]
#[case::neg_inf_ui(Amount::NegativeInfinity, "-inf", ui_session_team_key())]
#[case::neg_inf_user(Amount::NegativeInfinity, "-inf", user())]
#[case::neg_inf_cli(Amount::NegativeInfinity, "-inf", cli_session())]
fn nonfinite_before_admin_or_ui(#[case] bad: Amount, #[case] text: &str, #[case] who: Caller) {
    let windows = [amount("5"), bad];
    let error = check_budget_ceilings(Some(&windows), None, who, false).unwrap_err();
    assert_eq!(
        error,
        DelegationError::NonFiniteBudget {
            amount: text.to_owned(),
        }
    );
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert!(matches!(error.detail(), DelegationDetail::Error(_)));
}

#[test]
fn nonfinite_first_offending() {
    let both = [Amount::Nan, Amount::PositiveInfinity];
    assert_eq!(
        check_budget_ceilings(Some(&both), Some(amount("1")), user(), false),
        Err(DelegationError::NonFiniteBudget {
            amount: "nan".to_owned(),
        })
    );
    let swapped = [Amount::PositiveInfinity, Amount::Nan];
    assert_eq!(
        check_budget_ceilings(Some(&swapped), Some(amount("1")), user(), false),
        Err(DelegationError::NonFiniteBudget {
            amount: "inf".to_owned(),
        })
    );
}

#[test]
fn budget_admin_ui_exemption() {
    let windows = [amount("25")];
    let ceiling = Some(amount("10"));
    assert_eq!(
        check_budget_ceilings(Some(&windows), ceiling, admin(), false),
        Ok(())
    );
    assert_eq!(
        check_budget_ceilings(Some(&windows), ceiling, ui_session_team_key(), false),
        Ok(())
    );
    // a CLI session token on a personal key is held to the ceiling
    assert_eq!(
        check_budget_ceilings(Some(&windows), ceiling, cli_session(), true),
        Err(DelegationError::AboveCeiling {
            amount: "25".to_owned(),
            ceiling: "10".to_owned(),
        })
    );
}

#[test]
fn cli_personal_window_denied() {
    let windows = [amount("5")];
    let error = check_budget_ceilings(Some(&windows), Some(amount("10")), cli_session(), false)
        .unwrap_err();
    assert_eq!(error, DelegationError::SessionTokenWithoutTeam);
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert!(
        matches!(error.detail(), DelegationDetail::Error(message) if message.contains("team_id"))
    );
    // the team requirement runs before the ceiling, so no ceiling still rejects
    assert_eq!(
        check_budget_ceilings(Some(&windows), None, cli_session(), false),
        Err(DelegationError::SessionTokenWithoutTeam)
    );
    // with a team the session caller is held to the ceiling, and small windows pass
    assert_eq!(
        check_budget_ceilings(Some(&windows), Some(amount("10")), cli_session(), true),
        Ok(())
    );
    let small = [amount("0"), amount("-1")];
    assert_eq!(
        check_budget_ceilings(Some(&small), Some(amount("0")), cli_session(), true),
        Ok(())
    );
}

#[test]
fn no_ceiling_unlimited() {
    let windows = [amount("1000000")];
    assert_eq!(
        check_budget_ceilings(Some(&windows), None, user(), false),
        Ok(())
    );
    // Python compares floats: NaN and +inf ceilings exceed no window, and -inf exceeds every one
    assert_eq!(
        check_budget_ceilings(Some(&windows), Some(Amount::Nan), user(), false),
        Ok(())
    );
    assert_eq!(
        check_budget_ceilings(
            Some(&windows),
            Some(Amount::PositiveInfinity),
            user(),
            false
        ),
        Ok(())
    );
    assert_eq!(
        check_budget_ceilings(
            Some(&windows),
            Some(Amount::NegativeInfinity),
            user(),
            false
        ),
        Err(DelegationError::AboveCeiling {
            amount: "1000000".to_owned(),
            ceiling: "-inf".to_owned(),
        })
    );
}

#[rstest]
#[case::below("9.99", true)]
#[case::equal("10", true)]
#[case::above("10.01", false)]
#[case::zero("0", true)]
#[case::negative("-5", true)]
fn ceiling_below_equal_above(#[case] window: &str, #[case] passes: bool) {
    let windows = [amount(window)];
    let result = check_budget_ceilings(Some(&windows), Some(amount("10")), user(), false);
    if passes {
        assert_eq!(result, Ok(()));
    } else {
        let error = result.unwrap_err();
        assert_eq!(
            error,
            DelegationError::AboveCeiling {
                amount: window.to_owned(),
                ceiling: "10".to_owned(),
            }
        );
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert!(matches!(error.detail(), DelegationDetail::Error(_)));
    }
}

#[test]
fn first_offending_window() {
    let windows = [amount("5"), amount("15"), amount("20")];
    let error =
        check_budget_ceilings(Some(&windows), Some(amount("10")), user(), false).unwrap_err();
    assert_eq!(
        error,
        DelegationError::AboveCeiling {
            amount: "15".to_owned(),
            ceiling: "10".to_owned(),
        }
    );
    let message = error.to_string();
    assert!(message.contains("15"));
    assert!(message.contains("10"));
    assert!(!message.contains("20"));
}
