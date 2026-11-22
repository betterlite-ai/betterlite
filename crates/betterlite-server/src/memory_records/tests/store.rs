use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;

use super::support::{FakeRows, caller, row};
use crate::memory_records::store::{
    MemoryLookupError, find_memory_for_caller, find_visible_memory_or_none,
};

#[tokio::test]
async fn scoped_lookup_keeps_key_inside_visibility() {
    let rows = FakeRows::new(Some(row("mem-1", "note", Some("alice"), None)));
    let customer = caller(Some("alice"), Some("team-a"), None);
    let found = find_memory_for_caller(&rows, "note", &customer)
        .await
        .unwrap();
    assert_eq!(found.memory_id, "mem-1");
    let seen = rows.filters();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        serde_json::to_string(&seen[0]).unwrap(),
        r#"{"AND":[{"key":"note"},{"OR":[{"user_id":"alice"},{"team_id":"team-a"}]}]}"#
    );
}

#[tokio::test]
async fn admin_lookup_reads_with_the_key_filter_alone() {
    let rows = FakeRows::new(Some(row("mem-1", "note", Some("alice"), None)));
    let admin = caller(Some("root"), None, Some(UserRole::ProxyAdmin));
    find_memory_for_caller(&rows, "note", &admin).await.unwrap();
    let seen = rows.filters();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        serde_json::to_string(&seen[0]).unwrap(),
        r#"{"key":"note"}"#
    );
}

#[tokio::test]
async fn identity_less_lookup_matches_nothing() {
    let rows = FakeRows::new(None);
    let anonymous = caller(None, None, None);
    match find_memory_for_caller(&rows, "note", &anonymous).await {
        Ok(_) => panic!("the store has no row to answer"),
        Err(MemoryLookupError::Rows(_)) => panic!("the store answered"),
        Err(MemoryLookupError::Http(error)) => {
            assert_eq!(error.status, StatusCode::NOT_FOUND);
        }
    }
    let seen = rows.filters();
    assert_eq!(
        serde_json::to_string(&seen[0]).unwrap(),
        r#"{"AND":[{"key":"note"},{"memory_id":"__no_match__"}]}"#
    );
}

#[tokio::test]
async fn missing_row_is_a_404() {
    let rows = FakeRows::new(None);
    let customer = caller(Some("alice"), None, None);
    match find_memory_for_caller(&rows, "note", &customer).await {
        Ok(_) => panic!("the store has no row to answer"),
        Err(MemoryLookupError::Rows(_)) => panic!("the store answered"),
        Err(MemoryLookupError::Http(error)) => {
            assert_eq!(error.status, StatusCode::NOT_FOUND);
            assert_eq!(error.detail, "Memory with key 'note' not found");
        }
    }
}

#[tokio::test]
async fn visible_lookup_returns_the_row() {
    let rows = FakeRows::new(Some(row("mem-1", "note", None, Some("team-a"))));
    let teammate = caller(None, Some("team-a"), None);
    let found = find_visible_memory_or_none(&rows, "note", &teammate)
        .await
        .unwrap();
    assert_eq!(
        found.map(|stored| stored.memory_id),
        Some("mem-1".to_owned())
    );
}
