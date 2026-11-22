use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use rstest::rstest;

use super::super::responses::NO_DATABASE;
use super::support::{
    DetailEnvelope, ErrorEnvelope, FakeFailure, KeyBody, UserBody, admin, aliased, call, decimal,
    internal, internal_without_user, key, key_principal, master_key, org_admin, tag_store, user,
    viewer,
};

const FORBIDDEN_USER: &str = "Not authorized to view spend for another user.";
const NO_DB_CONNECTED: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";
const UNDECODABLE_ROW: &str = "the stored row is not a JSON object";

#[rstest]
#[case::keys("/spend/keys")]
#[case::users("/spend/users")]
#[case::tags("/spend/tags")]
#[case::tag_names("/global/spend/all_tag_names")]
#[case::daily_tags("/global/spend/tags?start_date=2024-01-01&end_date=2024-02-01")]
#[tokio::test]
async fn refuses_a_request_without_an_http_identity(#[case] uri: &str) {
    let store = Arc::new(tag_store());
    let reply = call(uri, None, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let envelope: ErrorEnvelope = reply.decode();
    assert_eq!(envelope.error.message, "the API key is not valid");
    assert_eq!(envelope.error.kind, "auth_error");
    assert_eq!(envelope.error.param.as_deref(), Some("None"));
    assert_eq!(envelope.error.code, "401");
}

#[rstest]
#[case::admin(admin())]
#[case::viewer(viewer())]
#[case::master_key(master_key())]
#[case::key_viewer(key_principal(r#"{"user_id":"u1","user_role":"proxy_admin_viewer"}"#))]
#[tokio::test]
async fn lists_every_key_for_an_admin_view(#[case] caller: Principal) {
    let mut fake = tag_store();
    fake.keys = vec![
        key(
            r#"{"token":"sk-b","key_alias":"b","key_name":"primary","user_id":"u2","spend":5,"litellm_budget_table":{"budget_id":"budget-1"}}"#,
        ),
        key(r#"{"token":"sk-a","user_id":"u1","spend":1}"#),
    ];
    let store = Arc::new(fake);
    let reply = call("/spend/keys", Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let rows: Vec<KeyBody> = reply.decode();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].token.as_deref(), Some("sk-b"));
    assert_eq!(rows[0].key_alias.as_deref(), Some("b"));
    assert_eq!(rows[0].key_name.as_deref(), Some("primary"));
    assert_eq!(rows[0].user_id.as_deref(), Some("u2"));
    assert_eq!(
        rows[0].litellm_budget_table.as_ref().unwrap().budget_id,
        "budget-1"
    );
    assert_eq!(rows[1].token.as_deref(), Some("sk-a"));
    assert_eq!(rows[1].user_id.as_deref(), Some("u1"));
    assert!(rows[1].key_name.is_none());
    assert!(rows[1].litellm_budget_table.is_none());
    assert_eq!(store.calls(), vec!["keys".to_owned()]);
}

#[rstest]
#[case::internal(internal("u1"))]
#[case::org_admin(org_admin("u1"))]
#[case::key_owner(key_principal(r#"{"user_id":"u1","user_role":"internal_user"}"#))]
#[tokio::test]
async fn scopes_a_non_admin_to_their_own_keys(#[case] caller: Principal) {
    let mut fake = tag_store();
    fake.user_keys = vec![key(r#"{"token":"sk-a","user_id":"u1","spend":2}"#)];
    let store = Arc::new(fake);
    let reply = call("/spend/keys", Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let rows: Vec<KeyBody> = reply.decode();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token.as_deref(), Some("sk-a"));
    assert_eq!(store.calls(), vec!["keys_for_user(u1)".to_owned()]);
}

#[rstest]
#[case::no_user(internal_without_user())]
#[case::empty_user(internal(""))]
#[tokio::test]
async fn returns_no_keys_for_a_falsey_caller_id(#[case] caller: Principal) {
    let mut fake = tag_store();
    fake.keys = vec![key(r#"{"token":"sk-a","user_id":"u1"}"#)];
    let store = Arc::new(fake);
    let reply = call("/spend/keys", Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "[]");
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::no_database(FakeFailure::NoDatabase, NO_DB_CONNECTED)]
#[case::undecodable(FakeFailure::Undecodable, UNDECODABLE_ROW)]
#[tokio::test]
async fn answers_a_key_store_failure_with_a_bad_request(
    #[case] failure: FakeFailure,
    #[case] message: &str,
) {
    let mut fake = tag_store();
    fake.failure = Some(failure);
    let store = Arc::new(fake);
    let reply = call("/spend/keys", Some(&admin()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let envelope: DetailEnvelope = reply.decode();
    assert_eq!(envelope.detail.error, message);
}

#[rstest]
#[case::keys("/spend/keys")]
#[case::users("/spend/users")]
#[tokio::test]
async fn answers_a_missing_database_with_a_bad_request(#[case] uri: &str) {
    let reply = call(uri, Some(&admin()), None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let envelope: DetailEnvelope = reply.decode();
    assert_eq!(envelope.detail.error, NO_DATABASE);
}

#[tokio::test]
async fn lists_every_user_with_the_key_aliases_for_an_admin() {
    let mut fake = tag_store();
    fake.users = vec![
        aliased(
            user(r#"{"user_id":"u2","spend":5,"password":"hunter2"}"#),
            &[None],
        ),
        aliased(
            user(r#"{"user_id":"u1","spend":1}"#),
            &[Some("primary"), None],
        ),
    ];
    let store = Arc::new(fake);
    let reply = call("/spend/users", Some(&admin()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.text();
    assert!(!body.contains("\"password\":"));
    assert!(!body.contains("hunter2"));
    let rows: Vec<UserBody> = reply.decode();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].user_id, "u2");
    assert_eq!(rows[0].key_aliases, Some(vec![None]));
    assert!(rows[0].organization_memberships.is_none());
    assert_eq!(
        rows[1].key_aliases,
        Some(vec![Some("primary".to_owned()), None])
    );
    assert_eq!(store.calls(), vec!["users".to_owned()]);
}

#[tokio::test]
async fn reads_one_user_for_an_admin_without_the_alias_join() {
    let mut fake = tag_store();
    fake.user = Some(user(
        r#"{"user_id":"u1","spend":2.5,"password":"hunter2","organization_memberships":[{"user_id":"u1","organization_id":"org-1","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}]}"#,
    ));
    let store = Arc::new(fake);
    let reply = call("/spend/users?user_id=u1", Some(&admin()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.text();
    assert!(!body.contains("\"password\":"));
    assert!(!body.contains("hunter2"));
    assert!(!body.contains("key_aliases"));
    let rows: Vec<Option<UserBody>> = reply.decode();
    assert_eq!(rows.len(), 1);
    let found = rows[0].as_ref().unwrap();
    assert_eq!(found.user_id, "u1");
    assert_eq!(found.spend, decimal("2.5"));
    let memberships = found.organization_memberships.as_ref().unwrap();
    assert_eq!(memberships[0].organization_id, "org-1");
    assert_eq!(store.calls(), vec!["user(u1)".to_owned()]);
}

#[tokio::test]
async fn returns_a_singleton_null_for_an_unknown_user() {
    let store = Arc::new(tag_store());
    let reply = call("/spend/users?user_id=missing", Some(&admin()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "[null]");
    assert_eq!(store.calls(), vec!["user(missing)".to_owned()]);
}

#[tokio::test]
async fn reads_an_empty_user_id_as_a_specific_missing_user() {
    let mut fake = tag_store();
    fake.users = vec![aliased(user(r#"{"user_id":"u1","spend":1}"#), &[None])];
    let store = Arc::new(fake);
    let reply = call("/spend/users?user_id=", Some(&admin()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "[null]");
    assert_eq!(store.calls(), vec!["user()".to_owned()]);
}

#[tokio::test]
async fn scopes_a_non_admin_to_their_own_user_row() {
    let mut fake = tag_store();
    fake.user = Some(user(r#"{"user_id":"u1","spend":1}"#));
    let store = Arc::new(fake);
    let reply = call("/spend/users", Some(&internal("u1")), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let rows: Vec<Option<UserBody>> = reply.decode();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].as_ref().unwrap().user_id, "u1");
    assert_eq!(store.calls(), vec!["user(u1)".to_owned()]);
}

#[tokio::test]
async fn accepts_an_internal_caller_asking_for_their_own_id() {
    let mut fake = tag_store();
    fake.user = Some(user(r#"{"user_id":"u1","spend":1}"#));
    let store = Arc::new(fake);
    let reply = call(
        "/spend/users?user_id=u1",
        Some(&internal("u1")),
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec!["user(u1)".to_owned()]);
}

#[rstest]
#[case::other_user("u2")]
#[case::empty_id("")]
#[tokio::test]
async fn refuses_an_internal_caller_reading_another_user(#[case] requested: &str) {
    let mut fake = tag_store();
    fake.user = Some(user(r#"{"user_id":"u2","spend":1}"#));
    let store = Arc::new(fake);
    let uri = format!("/spend/users?user_id={requested}");
    let reply = call(&uri, Some(&internal("u1")), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let envelope: DetailEnvelope = reply.decode();
    assert_eq!(envelope.detail.error, FORBIDDEN_USER);
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::no_user(internal_without_user())]
#[case::empty_user(internal(""))]
#[tokio::test]
async fn returns_no_users_for_a_falsey_caller_id_even_with_a_query(#[case] caller: Principal) {
    let store = Arc::new(tag_store());
    let reply = call("/spend/users?user_id=u1", Some(&caller), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "[]");
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::no_database(FakeFailure::NoDatabase, NO_DB_CONNECTED)]
#[case::undecodable(FakeFailure::Undecodable, UNDECODABLE_ROW)]
#[tokio::test]
async fn answers_a_user_store_failure_with_a_bad_request(
    #[case] failure: FakeFailure,
    #[case] message: &str,
) {
    let mut fake = tag_store();
    fake.failure = Some(failure);
    let store = Arc::new(fake);
    let reply = call("/spend/users", Some(&admin()), Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let envelope: DetailEnvelope = reply.decode();
    assert_eq!(envelope.detail.error, message);
}
