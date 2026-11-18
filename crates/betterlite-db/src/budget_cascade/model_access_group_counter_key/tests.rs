#![cfg(test)]

use super::super::linked_rows::{LinkedKeys as _, ModelAccessGroupRow};

fn row() -> ModelAccessGroupRow {
    ModelAccessGroupRow {
        access_group_name: "group-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn model_access_group_counter_key_is_shared_with_the_enforcement_path() {
    assert_eq!(
        row().counter_key(),
        "spend:model_access_group:group-a".to_owned()
    );
}

#[test]
fn model_access_group_cache_keys_use_the_registry_row_key() {
    assert_eq!(
        row().cache_keys(),
        vec!["model_access_group:group-a".to_owned()]
    );
}
