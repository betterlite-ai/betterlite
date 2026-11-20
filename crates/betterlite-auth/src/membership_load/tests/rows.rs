#![cfg(test)]

use betterlite_db::postgres_store::rows::{KeyColumn, Table as _};

use crate::membership_load::rows::MembershipRows;

#[test]
fn membership_query_is_keyed_by_user() {
    let query = MembershipRows.membership(KeyColumn::Id);
    assert!(query.contains("LiteLLM_TeamMembership"), "{query}");
    assert!(query.contains("user_id"), "{query}");
}
