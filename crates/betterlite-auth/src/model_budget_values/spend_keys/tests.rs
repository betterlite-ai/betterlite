#![cfg(test)]
use rstest::rstest;

use super::model_budget_spend_cache_key;
use crate::model_budget_values::budget_scope::BudgetScope;

#[rstest]
#[case::key(BudgetScope::Key, "virtual_key_spend:id-1:gpt-4o:1d")]
#[case::user(BudgetScope::User, "user_model_spend:id-1:gpt-4o:1d")]
#[case::end_user(BudgetScope::EndUser, "end_user_model_spend:id-1:gpt-4o:1d")]
#[case::team(BudgetScope::Team, "team_model_spend:id-1:gpt-4o:1d")]
fn spend_keys_use_the_scope_prefix(#[case] scope: BudgetScope, #[case] expected: &str) {
    assert_eq!(
        model_budget_spend_cache_key(scope, "id-1", "gpt-4o", "1d"),
        expected
    );
}

#[test]
fn spend_keys_keep_the_budget_model_verbatim() {
    assert_eq!(
        model_budget_spend_cache_key(BudgetScope::Key, "k", "openai/gpt-4o:x", "1h"),
        "virtual_key_spend:k:openai/gpt-4o:x:1h"
    );
}

#[test]
fn spend_keys_order_the_entity_before_the_model_before_the_duration() {
    assert_eq!(
        model_budget_spend_cache_key(BudgetScope::Team, "entity", "model", "duration"),
        "team_model_spend:entity:model:duration"
    );
}
