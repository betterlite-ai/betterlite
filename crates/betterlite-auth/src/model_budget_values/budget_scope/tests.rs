#![cfg(test)]
use rstest::rstest;

use super::{BudgetPrefixes, BudgetScope};

#[rstest]
#[case::key(
    BudgetScope::Key,
    BudgetPrefixes {
        spend: "virtual_key_spend",
        start_time: "virtual_key_budget_start_time",
    }
)]
#[case::user(
    BudgetScope::User,
    BudgetPrefixes {
        spend: "user_model_spend",
        start_time: "user_model_budget_start_time",
    }
)]
#[case::end_user(
    BudgetScope::EndUser,
    BudgetPrefixes {
        spend: "end_user_model_spend",
        start_time: "end_user_budget_start_time",
    }
)]
#[case::team(
    BudgetScope::Team,
    BudgetPrefixes {
        spend: "team_model_spend",
        start_time: "team_model_budget_start_time",
    }
)]
fn budget_scope_names_both_key_prefixes(
    #[case] scope: BudgetScope,
    #[case] expected: BudgetPrefixes,
) {
    assert_eq!(scope.prefixes(), expected);
}
