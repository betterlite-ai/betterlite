#![cfg(test)]

use super::super::linked_rows::{LinkedKeys as _, TagRow};

fn row() -> TagRow {
    TagRow {
        tag_name: "team-blue".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn tag_counter_key_wraps_the_tag_under_spend_tag() {
    assert_eq!(row().counter_key(), "spend:tag:team-blue".to_owned());
}

#[test]
fn tag_cache_keys_use_the_tag_cache_key_format() {
    assert_eq!(row().cache_keys(), vec!["tag:team-blue".to_owned()]);
}
