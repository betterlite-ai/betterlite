#![cfg(test)]
use rstest::rstest;

use super::model_budget_start_time_cache_key;
use crate::model_budget_values::budget_scope::BudgetScope;

#[rstest]
#[case::key(BudgetScope::Key, "virtual_key_budget_start_time:id-1:gpt-4o:30d")]
#[case::user(BudgetScope::User, "user_model_budget_start_time:id-1:gpt-4o:30d")]
#[case::end_user(BudgetScope::EndUser, "end_user_budget_start_time:id-1:gpt-4o:30d")]
#[case::team(BudgetScope::Team, "team_model_budget_start_time:id-1:gpt-4o:30d")]
fn start_time_keys_use_the_scope_prefix(#[case] scope: BudgetScope, #[case] expected: &str) {
    assert_eq!(
        model_budget_start_time_cache_key(scope, "id-1", "gpt-4o", "30d"),
        expected
    );
}

#[test]
fn start_time_keys_keep_the_budget_model_verbatim() {
    assert_eq!(
        model_budget_start_time_cache_key(BudgetScope::User, "u", "openai/gpt-4o:x", "1h"),
        "user_model_budget_start_time:u:openai/gpt-4o:x:1h"
    );
}

#[test]
fn start_time_keys_order_the_entity_before_the_model_before_the_duration() {
    assert_eq!(
        model_budget_start_time_cache_key(BudgetScope::Team, "entity", "model", "duration"),
        "team_model_budget_start_time:entity:model:duration"
    );
}
