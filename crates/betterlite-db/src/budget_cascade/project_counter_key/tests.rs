#![cfg(test)]

use super::super::linked_rows::{LinkedKeys as _, ProjectRow};

fn row() -> ProjectRow {
    ProjectRow {
        project_id: "project-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn project_counter_key_wraps_the_project_under_spend_project() {
    assert_eq!(row().counter_key(), "spend:project:project-a".to_owned());
}

#[test]
fn project_cache_keys_use_the_project_id_key() {
    assert_eq!(row().cache_keys(), vec!["project_id:project-a".to_owned()]);
}
