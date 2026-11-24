use betterlite_db::budget_permission_rows::JsonData;

use super::support::{FakeStore, params, strings, team_caller, user_row};
use crate::key_list_filters::alias_scope::apply_non_admin_alias_scope;
use crate::team_access::caller::TeamCaller;

async fn bind(
    caller: &TeamCaller,
    store: &FakeStore,
    query: Vec<JsonData>,
    filters: Vec<String>,
) -> (Vec<JsonData>, Vec<String>) {
    let mut bound_params = query;
    let mut bound_filters = filters;
    apply_non_admin_alias_scope(caller, store, &mut bound_params, &mut bound_filters)
        .await
        .unwrap();
    (bound_params, bound_filters)
}

#[tokio::test]
async fn aliases_existing_bind_offsets() {
    let store = FakeStore::with_user(user_row(r#"{"user_id":"u1","teams":["t1","t2"]}"#));
    let owner = team_caller(Some("u1"));
    let start = vec!["user_id = $1".to_owned()];
    let (query, filters) = bind(&owner, &store, params(&["keep"]), start).await;
    assert_eq!(query, params(&["keep", "u1", "t1", "t2"]));
    assert_eq!(
        filters,
        strings(&["user_id = $1", "(user_id = $2 OR team_id IN ($3, $4))"])
    );
    assert_eq!(store.reads(), strings(&["user u1"]));
}

#[tokio::test]
async fn aliases_owner_or_user_row_teams() {
    let store = FakeStore::with_user(user_row(r#"{"user_id":"u1","teams":["t1","t2"]}"#));
    let owner = team_caller(Some("u1"));
    let (query, filters) = bind(&owner, &store, Vec::new(), Vec::new()).await;
    assert_eq!(query, params(&["u1", "t1", "t2"]));
    assert_eq!(filters, strings(&["(user_id = $1 OR team_id IN ($2, $3))"]));
}

#[tokio::test]
async fn aliases_missing_row_owner_only() {
    let store = FakeStore::default();
    let owner = team_caller(Some("u1"));
    let (query, filters) = bind(&owner, &store, Vec::new(), Vec::new()).await;
    assert_eq!(query, params(&["u1"]));
    assert_eq!(filters, strings(&["(user_id = $1)"]));
    assert_eq!(store.reads(), strings(&["user u1"]));
    let empty_teams = FakeStore::with_user(user_row(r#"{"user_id":"u1","teams":[]}"#));
    let (empty_query, empty_filters) = bind(&owner, &empty_teams, Vec::new(), Vec::new()).await;
    assert_eq!(empty_query, params(&["u1"]));
    assert_eq!(empty_filters, strings(&["(user_id = $1)"]));
}

#[tokio::test]
async fn aliases_missing_identity_false() {
    let store = FakeStore::with_user(user_row(r#"{"user_id":"u1","teams":["t1"]}"#));
    let anonymous = team_caller(None);
    let (query, filters) = bind(&anonymous, &store, params(&["keep"]), Vec::new()).await;
    assert_eq!(query, params(&["keep"]));
    assert_eq!(filters, strings(&["FALSE"]));
    assert!(store.reads().is_empty());
    let empty_identity = team_caller(Some(""));
    let (blank_query, blank_filters) = bind(&empty_identity, &store, Vec::new(), Vec::new()).await;
    assert!(blank_query.is_empty());
    assert_eq!(blank_filters, strings(&["FALSE"]));
    assert!(store.reads().is_empty());
}

#[tokio::test]
async fn aliases_hostile_identifier_bound() {
    let hostile_user = "u1'; DROP TABLE \"LiteLLM_UserTable\" --";
    let hostile_team = "t1'); DELETE FROM \"LiteLLM_VerificationToken\"; --";
    let stored = serde_json::to_string(hostile_team).unwrap();
    let store = FakeStore::with_user(user_row(&format!(
        r#"{{"user_id":"u1","teams":[{stored}]}}"#
    )));
    let owner = team_caller(Some(hostile_user));
    let (query, filters) = bind(&owner, &store, Vec::new(), Vec::new()).await;
    assert_eq!(query, params(&[hostile_user, hostile_team]));
    assert_eq!(filters, strings(&["(user_id = $1 OR team_id IN ($2))"]));
    assert!(!filters[0].contains("DROP"));
    assert!(!filters[0].contains("DELETE"));
}
