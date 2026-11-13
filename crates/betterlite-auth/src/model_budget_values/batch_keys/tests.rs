#![cfg(test)]
use super::batch_charged_once_marker_key;
use crate::model_budget_values::budget_scope::BudgetScope;
use crate::model_budget_values::spend_keys::model_budget_spend_cache_key;

#[test]
fn batch_keys_append_the_batch_id_to_the_spend_key() {
    let spend_key = model_budget_spend_cache_key(BudgetScope::Key, "k", "gpt-4o", "1d");
    assert_eq!(
        batch_charged_once_marker_key(&spend_key, "batch_abc"),
        "virtual_key_spend:k:gpt-4o:1d:batch:batch_abc"
    );
}
