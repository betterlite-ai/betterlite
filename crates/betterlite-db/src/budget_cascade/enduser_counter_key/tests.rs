#![cfg(test)]

use super::super::linked_rows::{EndUserRow, LinkedKeys as _};

fn row() -> EndUserRow {
    EndUserRow {
        user_id: "customer-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn enduser_counter_key_wraps_the_customer_under_spend_end_user() {
    assert_eq!(row().counter_key(), "spend:end_user:customer-a".to_owned());
}

#[test]
fn enduser_cache_keys_use_the_end_user_id_key() {
    assert_eq!(
        row().cache_keys(),
        vec!["end_user_id:customer-a".to_owned()]
    );
}
