#![cfg(test)]

mod postgres;
mod support;

use std::str::from_utf8;
use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use rstest::rstest;

use self::support::{admin, alice, caller, user, visibility};
use super::error::VisibilityError;
use super::owners::{OwnerPair, cold_payload_owner};
use super::postgres::PostgresVisibility;
use super::query::{SpendLogViewer, payload_query};
use super::roles::{can_user_view_spend_log, is_admin_view_safe};

const ADMIN_SQL: &str = r#"SELECT request_id, messages, response, proxy_server_request, metadata, "user", team_id FROM "LiteLLM_SpendLogs" WHERE (request_id = $1 OR litellm_call_id = $1) ORDER BY (request_id = $1) DESC LIMIT 1"#;
const OWNER_SQL: &str = r#"SELECT request_id, messages, response, proxy_server_request, metadata, "user", team_id FROM "LiteLLM_SpendLogs" WHERE (request_id = $1 OR litellm_call_id = $1) AND "user" = $2 ORDER BY (request_id = $1) DESC LIMIT 1"#;
const TEAM_SQL: &str = r#"SELECT request_id, messages, response, proxy_server_request, metadata, "user", team_id FROM "LiteLLM_SpendLogs" WHERE (request_id = $1 OR litellm_call_id = $1) AND ("user" = $2 OR team_id = ANY($3::text[])) ORDER BY (request_id = $1) DESC LIMIT 1"#;

#[rstest]
#[case::proxy_admin(Some(UserRole::ProxyAdmin), true)]
#[case::proxy_admin_viewer(Some(UserRole::ProxyAdminViewer), true)]
#[case::org_admin(Some(UserRole::OrgAdmin), false)]
#[case::internal_user(Some(UserRole::InternalUser), false)]
#[case::internal_user_viewer(Some(UserRole::InternalUserViewer), false)]
#[case::team(Some(UserRole::Team), false)]
#[case::customer(Some(UserRole::Customer), false)]
#[case::no_role(None, false)]
fn only_proxy_admins_see_every_spend_log(#[case] role: Option<UserRole>, #[case] expected: bool) {
    assert_eq!(is_admin_view_safe(&caller(Some("u1"), role)), expected);
}

#[rstest]
#[case::internal_user(Some("u1"), Some(UserRole::InternalUser), true)]
#[case::internal_user_viewer(Some("u1"), Some(UserRole::InternalUserViewer), true)]
#[case::internal_user_without_id(None, Some(UserRole::InternalUser), false)]
#[case::viewer_without_id(None, Some(UserRole::InternalUserViewer), false)]
#[case::proxy_admin(Some("u1"), Some(UserRole::ProxyAdmin), false)]
#[case::proxy_admin_viewer(Some("u1"), Some(UserRole::ProxyAdminViewer), false)]
#[case::org_admin(Some("u1"), Some(UserRole::OrgAdmin), false)]
#[case::team(Some("u1"), Some(UserRole::Team), false)]
#[case::customer(Some("u1"), Some(UserRole::Customer), false)]
#[case::no_role(Some("u1"), None, false)]
fn own_list_scope_needs_an_internal_role_and_a_user(
    #[case] user_id: Option<&str>,
    #[case] role: Option<UserRole>,
    #[case] expected: bool,
) {
    assert_eq!(can_user_view_spend_log(&caller(user_id, role)), expected);
}

#[rstest]
#[case::admin(None, ADMIN_SQL, r#"["id-1"]"#, 1)]
#[case::owner(Some(SpendLogViewer { user_id: Some("u1".to_owned()), team_ids: Vec::new() }), OWNER_SQL, r#"["id-1","u1"]"#, 2)]
#[case::null_user(Some(SpendLogViewer { user_id: None, team_ids: Vec::new() }), OWNER_SQL, r#"["id-1",null]"#, 2)]
#[case::teams(Some(SpendLogViewer { user_id: Some("u1".to_owned()), team_ids: vec!["t1".to_owned(), "t2".to_owned()] }), TEAM_SQL, r#"["id-1","u1",["t1","t2"]]"#, 3)]
#[case::null_user_teams(Some(SpendLogViewer { user_id: None, team_ids: vec!["t1".to_owned()] }), TEAM_SQL, r#"["id-1",null,["t1"]]"#, 3)]
fn payload_query_scopes_every_nonadmin(
    #[case] viewer: Option<SpendLogViewer>,
    #[case] sql: &str,
    #[case] binds: &str,
    #[case] bound: usize,
) {
    let query = payload_query("id-1", viewer.as_ref());
    assert_eq!(query.sql, sql);
    assert_eq!(serde_json::to_string(&query.binds).unwrap(), binds);
    assert_eq!(query.params().len(), bound);
}

#[rstest]
#[case::both(
    r#"{"metadata":{"user_api_key_user_id":"u1","user_api_key_team_id":"t1"}}"#,
    Some("u1"),
    Some("t1")
)]
#[case::empty_strings(
    r#"{"metadata":{"user_api_key_user_id":"","user_api_key_team_id":""}}"#,
    Some(""),
    Some("")
)]
#[case::team_only(r#"{"metadata":{"user_api_key_team_id":"t1"}}"#, None, Some("t1"))]
#[case::user_only(r#"{"metadata":{"user_api_key_user_id":"u1"}}"#, Some("u1"), None)]
#[case::not_strings(
    r#"{"metadata":{"user_api_key_user_id":7,"user_api_key_team_id":["t1"]}}"#,
    None,
    None
)]
#[case::nulls(
    r#"{"metadata":{"user_api_key_user_id":null,"user_api_key_team_id":null}}"#,
    None,
    None
)]
#[case::metadata_text(r#"{"metadata":"u1"}"#, None, None)]
#[case::top_level_only(r#"{"user_api_key_user_id":"u1"}"#, None, None)]
#[case::not_an_object(r#"["u1"]"#, None, None)]
fn cold_payload_owner_reads_string_metadata_only(
    #[case] payload: &str,
    #[case] owner_user: Option<&str>,
    #[case] owner_team: Option<&str>,
) {
    let parsed: JsonData = serde_json::from_str(payload).unwrap();
    let expected = OwnerPair {
        user: owner_user.map(str::to_owned),
        team_id: owner_team.map(str::to_owned),
    };
    assert_eq!(cold_payload_owner(&parsed), expected);
}

#[tokio::test]
async fn forbidden_keeps_the_source_detail_object() {
    let response = VisibilityError::Forbidden {
        request_id: "id-1".to_owned(),
    }
    .into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        from_utf8(&body).unwrap(),
        r#"{"detail":{"error":"Not authorized to view spend log for request_id=id-1"}}"#
    );
}

#[tokio::test]
async fn read_failures_answer_500_not_403() {
    let response = VisibilityError::from(DbError::NoDbConnected).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn an_unreadable_table_fails_instead_of_passing() {
    let store = PostgresVisibility {
        store: PostgresStore { pool: None },
    };
    let checks = visibility(Arc::new(store), vec![user("alice", &["team-a"])]);
    let precheck = checks.assert_can_view_request_id(&alice(), "id-1").await;
    assert!(matches!(precheck, Err(VisibilityError::Database(_))));
    let resolved = checks.resolve_payload_row(&alice(), "id-1").await;
    assert!(matches!(resolved, Err(VisibilityError::Database(_))));
    let teams = checks.permitted_team_ids(&alice()).await;
    assert!(matches!(teams, Err(VisibilityError::TeamList(_))));
    let cold = OwnerPair {
        user: None,
        team_id: Some("team-a".to_owned()),
    };
    let owner = checks.can_view_owner(&alice(), &cold).await;
    assert!(matches!(owner, Err(VisibilityError::Team(_))));
    checks
        .assert_can_view_request_id(&admin(), "id-1")
        .await
        .unwrap();
    checks
        .assert_owns_cold_payload(&admin(), &JsonData::Null, "id-1")
        .await
        .unwrap();
}
