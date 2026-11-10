#![cfg(test)]
use rstest::rstest;

use super::{
    BudgetScope, batch_charged_once_marker_key, model_budget_spend_cache_key,
    model_budget_start_time_cache_key,
};

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

#[rstest]
#[case::key(BudgetScope::Key, "virtual_key_budget_start_time:id-1:gpt-4o:30d")]
#[case::user(BudgetScope::User, "user_model_budget_start_time:id-1:gpt-4o:30d")]
#[case::end_user(BudgetScope::EndUser, "end_user_budget_start_time:id-1:gpt-4o:30d")]
#[case::team(BudgetScope::Team, "team_model_budget_start_time:id-1:gpt-4o:30d")]
fn cache_keys_for_start_times_use_the_scope_prefix(
    #[case] scope: BudgetScope,
    #[case] expected: &str,
) {
    assert_eq!(
        model_budget_start_time_cache_key(scope, "id-1", "gpt-4o", "30d"),
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
fn cache_keys_order_the_entity_before_the_model_before_the_duration() {
    assert_eq!(
        model_budget_spend_cache_key(BudgetScope::Team, "entity", "model", "duration"),
        "team_model_spend:entity:model:duration"
    );
    assert_eq!(
        model_budget_start_time_cache_key(BudgetScope::Team, "entity", "model", "duration"),
        "team_model_budget_start_time:entity:model:duration"
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
fn batch_keys_append_the_batch_id_to_the_spend_key() {
    let spend_key = model_budget_spend_cache_key(BudgetScope::Key, "k", "gpt-4o", "1d");
    assert_eq!(
        batch_charged_once_marker_key(&spend_key, "batch_abc"),
        "virtual_key_spend:k:gpt-4o:1d:batch:batch_abc"
    );
}
