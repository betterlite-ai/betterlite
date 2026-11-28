#![cfg(test)]

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    FakeScope, FakeStore, ScopeRights, internal_user, invitation_router, key_caller, master,
    org_admin, parse, platform_admin, platform_viewer, record, router_for, send, userless_admin,
};
use crate::auth::auth_key_lookup::tests::caller_through_key_stage;
use crate::invitation_routes::state::{
    InvitationDatabase, InvitationRouteState, InvitationScope, QueueRouteState,
};
use crate::invitation_routes::store::PostgresInvitations;
use crate::user_invitation::{InvitationError, InvitationRecord};

const DB_NOT_CONNECTED: &str = CommonProxyErrors::DbNotConnectedError.as_str();
const INTERNAL_SERVER_ERROR_BODY: &str = r#"{"error":{"message":"Internal server error","type":"internal_server_error","param":null,"code":"500"}}"#;
const NOT_ALLOWED: &str = CommonProxyErrors::NotAllowedAccess.as_str();

fn setup(
    records: Vec<InvitationRecord>,
    scope: FakeScope,
) -> (Arc<FakeStore>, Arc<FakeScope>, Router) {
    let store = Arc::new(FakeStore::with_records(records));
    let shared_scope = Arc::new(scope);
    let router = router_for(invitation_router(&store, &shared_scope));
    (store, shared_scope, router)
}

fn open_scope() -> FakeScope {
    FakeScope::new(ScopeRights::Open)
}

#[rstest]
#[case::new("POST", "/invitation/new", r#"{"user_id":"alice"}"#)]
#[case::info("GET", "/invitation/info?invitation_id=inv-1", "")]
#[case::update(
    "POST",
    "/invitation/update",
    r#"{"invitation_id":"inv-1","is_accepted":true}"#
)]
#[case::delete("POST", "/invitation/delete", r#"{"invitation_id":"inv-1"}"#)]
#[case::queue("POST", "/queue/chat/completions", r#"{"priority":1}"#)]
#[tokio::test]
async fn invitation_routes_need_a_caller(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let (_, _, router) = setup(Vec::new(), open_scope());
    let (status, _, text) = Box::pin(send(&router, method, uri, body, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    assert!(parse(&text)["error"]["message"].is_string(), "{uri}");
}

#[tokio::test]
async fn new_invitation_requires_the_database() {
    let router = router_for(InvitationRouteState::default());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"alice"}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let body = parse(&text);
    assert_eq!(body["error"]["message"], DB_NOT_CONNECTED);
    assert_eq!(body["error"]["type"], "internal_server_error");
    assert!(body["error"]["param"].is_null());
    assert_eq!(body["error"]["code"], "400");
}

#[tokio::test]
async fn new_invitation_refuses_a_caller_without_admin_privileges() {
    let (store, scope, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Refused));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"alice"}"#,
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let body = parse(&text);
    assert_eq!(
        body["error"]["message"],
        format!("{NOT_ALLOWED}, your role=internal_user")
    );
    assert_eq!(body["error"]["type"], "internal_server_error");
    assert_eq!(body["error"]["code"], "400");
    assert_eq!(scope.calls(), vec!["has_admin_privileges".to_owned()]);
    assert!(
        store
            .rows
            .lock()
            .expect("the store lock")
            .created
            .is_empty()
    );
}

#[tokio::test]
async fn new_invitation_by_a_proxy_admin_skips_the_scope_checks() {
    let (store, scope, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Refused));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"alice"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["id"], "fresh-id");
    assert_eq!(parse(&text)["user_id"], "alice");
    assert!(scope.calls().is_empty());
    let rows = store.rows.lock().expect("the store lock");
    assert_eq!(
        rows.created,
        vec![("alice".to_owned(), Some("admin-user".to_owned()))]
    );
}

#[tokio::test]
async fn new_invitation_by_the_master_key_has_no_actor() {
    let (store, scope, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Refused));
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"alice"}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(scope.calls().is_empty());
    let rows = store.rows.lock().expect("the store lock");
    assert_eq!(rows.created, vec![("alice".to_owned(), None)]);
}

#[tokio::test]
async fn new_invitation_by_an_org_admin_checks_the_target() {
    let (store, scope, router) = setup(Vec::new(), open_scope());
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"alice"}"#,
        Some(org_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        scope.calls(),
        vec![
            "has_admin_privileges".to_owned(),
            "can_invite_user:alice".to_owned()
        ]
    );
    let rows = store.rows.lock().expect("the store lock");
    assert_eq!(
        rows.created,
        vec![("alice".to_owned(), Some("org-admin".to_owned()))]
    );
}

#[tokio::test]
async fn new_invitation_by_an_org_admin_key_records_its_user() {
    let (store, scope, router) = setup(Vec::new(), open_scope());
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"alice"}"#,
        Some(caller_through_key_stage(
            r#"{"user_id":"u1","user_role":"org_admin"}"#,
        )),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        scope.calls(),
        vec![
            "has_admin_privileges".to_owned(),
            "can_invite_user:alice".to_owned()
        ]
    );
    let rows = store.rows.lock().expect("the store lock");
    assert_eq!(
        rows.created,
        vec![("alice".to_owned(), Some("u1".to_owned()))]
    );
}

#[tokio::test]
async fn new_invitation_refuses_a_target_outside_the_scope() {
    let (store, scope, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Privileged));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"outsider"}"#,
        Some(org_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["error"]["message"],
        "You can only create invitations for users in your organization or team."
    );
    assert_eq!(
        scope.calls(),
        vec![
            "has_admin_privileges".to_owned(),
            "can_invite_user:outsider".to_owned()
        ]
    );
    assert!(
        store
            .rows
            .lock()
            .expect("the store lock")
            .created
            .is_empty()
    );
}

#[tokio::test]
async fn new_invitation_reports_a_missing_invitee() {
    let (store, _, router) = setup(Vec::new(), open_scope());
    store.rows.lock().expect("the store lock").create_error = Some(InvitationError::UserMissing);
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        r#"{"user_id":"ghost"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["error"]["message"],
        "User id does not exist in 'LiteLLM_UserTable'. Fix this by creating user via `/user/new`."
    );
    assert_eq!(parse(&text)["error"]["code"], "400");
}

#[tokio::test]
async fn new_invitation_rejects_a_body_without_user_id() {
    let (_, _, router) = setup(Vec::new(), open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        "{}",
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = &parse(&text)["detail"][0];
    assert_eq!(issue["type"], "missing");
    assert_eq!(issue["loc"], serde_json::json!(["body", "user_id"]));
    assert_eq!(issue["msg"], "Field required");
}

#[tokio::test]
async fn new_invitation_rejects_broken_json() {
    let (_, _, router) = setup(Vec::new(), open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/new",
        "{",
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parse(&text)["detail"][0]["type"], "json_invalid");
}

#[tokio::test]
async fn info_requires_the_invitation_id() {
    let (_, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info",
        "",
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let issue = &parse(&text)["detail"][0];
    assert_eq!(issue["type"], "missing");
    assert_eq!(issue["loc"], serde_json::json!(["query", "invitation_id"]));
    assert_eq!(issue["msg"], "Field required");
}

#[tokio::test]
async fn info_requires_an_admin_view() {
    let (_, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["detail"]["error"],
        format!("{NOT_ALLOWED}, your role=internal_user")
    );
}

#[tokio::test]
async fn info_answers_a_platform_viewer() {
    let (_, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(platform_viewer()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["id"], "inv-1");
    assert_eq!(parse(&text)["user_id"], "alice");
    assert_eq!(parse(&text)["created_by"], "admin-user");
    assert!(!parse(&text)["is_accepted"].as_bool().expect("a bool"));
    assert!(parse(&text)["accepted_at"].is_null());
}

#[tokio::test]
async fn info_answers_a_platform_admin() {
    let (_, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["id"], "inv-1");
}

#[tokio::test]
async fn info_answers_a_key_with_an_admin_view() {
    let (_, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let caller = key_caller(r#"{"user_role":"proxy_admin","user_id":"key-user"}"#);
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(caller),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["id"], "inv-1");
}

#[tokio::test]
async fn info_reports_a_missing_row() {
    let (_, _, router) = setup(Vec::new(), open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=ghost",
        "",
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["detail"]["error"],
        "Invitation id does not exist in the database."
    );
}

#[tokio::test]
async fn info_reports_a_store_failure() {
    let (store, _, router) = setup(Vec::new(), open_scope());
    store.rows.lock().expect("the store lock").read_error = Some(DbError::NoDbConnected);
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, INTERNAL_SERVER_ERROR_BODY);
}

#[tokio::test]
async fn info_requires_the_database() {
    let router = router_for(InvitationRouteState::default());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(parse(&text)["detail"]["error"], DB_NOT_CONNECTED);
}

#[tokio::test]
async fn info_checks_the_database_before_the_admin_view() {
    let router = router_for(InvitationRouteState::default());
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(parse(&text)["detail"]["error"], DB_NOT_CONNECTED);
}

#[tokio::test]
async fn info_reports_a_database_it_cannot_reach() {
    let scope: Arc<dyn InvitationScope> = Arc::new(open_scope());
    let store = PostgresInvitations {
        store: PostgresStore { pool: None },
    };
    let router = router_for(InvitationRouteState {
        invitations: Some(InvitationDatabase {
            invitations: Arc::new(store),
            scope,
        }),
        queue: QueueRouteState::default(),
    });
    let (status, _, text) = Box::pin(send(
        &router,
        "GET",
        "/invitation/info?invitation_id=inv-1",
        "",
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, INTERNAL_SERVER_ERROR_BODY);
}

#[tokio::test]
async fn update_needs_a_user_id() {
    let (store, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(userless_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        parse(&text)["detail"]["error"],
        "Unable to identify user id. Received=None"
    );
    assert!(
        store
            .rows
            .lock()
            .expect("the store lock")
            .updated
            .is_empty()
    );
}

#[tokio::test]
async fn update_accepts_the_invitation_from_the_master_key() {
    let (store, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["updated_by"], "default_user_id");
    assert_eq!(
        store.rows.lock().expect("the store lock").updated,
        vec![("inv-1".to_owned(), true, "default_user_id".to_owned())]
    );
}

#[tokio::test]
async fn update_accepts_the_invitation() {
    let (store, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(parse(&text)["is_accepted"].as_bool().expect("a bool"));
    assert_eq!(parse(&text)["updated_by"], "member");
    let rows = store.rows.lock().expect("the store lock");
    assert_eq!(
        rows.updated,
        vec![("inv-1".to_owned(), true, "member".to_owned())]
    );
}

#[tokio::test]
async fn update_by_a_user_bound_key_records_its_user() {
    let (store, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(caller_through_key_stage(r#"{"user_id":"u1"}"#)),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["updated_by"], "u1");
    assert_eq!(
        store.rows.lock().expect("the store lock").updated,
        vec![("inv-1".to_owned(), true, "u1".to_owned())]
    );
}

#[tokio::test]
async fn update_reports_a_missing_row() {
    let (_, _, router) = setup(Vec::new(), open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"ghost","is_accepted":false}"#,
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["detail"]["error"],
        "Invitation id does not exist in the database."
    );
}

#[tokio::test]
async fn update_reports_a_store_failure() {
    let (store, _, router) = setup(Vec::new(), open_scope());
    store.rows.lock().expect("the store lock").update_error = Some(DbError::NoDbConnected);
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, INTERNAL_SERVER_ERROR_BODY);
}

#[tokio::test]
async fn update_requires_the_database() {
    let router = router_for(InvitationRouteState::default());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(parse(&text)["detail"]["error"], DB_NOT_CONNECTED);
}

#[tokio::test]
async fn update_reports_a_database_it_cannot_reach() {
    let scope: Arc<dyn InvitationScope> = Arc::new(open_scope());
    let store = PostgresInvitations {
        store: PostgresStore { pool: None },
    };
    let router = router_for(InvitationRouteState {
        invitations: Some(InvitationDatabase {
            invitations: Arc::new(store),
            scope,
        }),
        queue: QueueRouteState::default(),
    });
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1","is_accepted":true}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, INTERNAL_SERVER_ERROR_BODY);
}

#[tokio::test]
async fn update_rejects_a_body_without_is_accepted() {
    let (_, _, router) = setup(vec![record("inv-1", "alice", "admin-user")], open_scope());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/update",
        r#"{"invitation_id":"inv-1"}"#,
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        parse(&text)["detail"][0]["loc"],
        serde_json::json!(["body", "is_accepted"])
    );
}

#[tokio::test]
async fn delete_lets_a_proxy_admin_delete_another_admins_invitation() {
    let (store, scope, router) = setup(
        vec![record("inv-1", "alice", "someone-else")],
        FakeScope::new(ScopeRights::Refused),
    );
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["id"], "inv-1");
    assert_eq!(scope.calls(), vec!["has_admin_privileges".to_owned()]);
    assert_eq!(
        store.rows.lock().expect("the store lock").deleted,
        vec!["inv-1".to_owned()]
    );
}

#[tokio::test]
async fn delete_lets_an_org_admin_delete_their_own_invitation() {
    let (store, _, router) = setup(
        vec![record("inv-1", "alice", "org-admin")],
        FakeScope::new(ScopeRights::Privileged),
    );
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(org_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store.rows.lock().expect("the store lock").deleted,
        vec!["inv-1".to_owned()]
    );
}

#[tokio::test]
async fn delete_lets_an_org_admin_key_delete_its_own_invitation() {
    let (store, _, router) = setup(
        vec![record("inv-1", "alice", "u1")],
        FakeScope::new(ScopeRights::Privileged),
    );
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(caller_through_key_stage(
            r#"{"user_id":"u1","user_role":"org_admin"}"#,
        )),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store.rows.lock().expect("the store lock").deleted,
        vec!["inv-1".to_owned()]
    );
}

#[tokio::test]
async fn delete_refuses_another_admins_invitation() {
    let (store, _, router) = setup(
        vec![record("inv-1", "alice", "other-admin")],
        FakeScope::new(ScopeRights::Privileged),
    );
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(org_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        parse(&text)["detail"]["error"],
        "Organization admins can only delete invitations they created."
    );
    assert!(
        store
            .rows
            .lock()
            .expect("the store lock")
            .deleted
            .is_empty()
    );
}

#[tokio::test]
async fn delete_refuses_a_caller_without_admin_privileges() {
    let (store, _, router) = setup(
        vec![record("inv-1", "alice", "member")],
        FakeScope::new(ScopeRights::Refused),
    );
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(internal_user()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["detail"]["error"],
        format!("{NOT_ALLOWED}, your role=internal_user")
    );
    assert!(
        store
            .rows
            .lock()
            .expect("the store lock")
            .deleted
            .is_empty()
    );
}

#[tokio::test]
async fn delete_reports_a_missing_row_before_the_ownership_check() {
    let (_, _, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Privileged));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"ghost"}"#,
        Some(org_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["detail"]["error"],
        "Invitation id does not exist in the database."
    );
}

#[tokio::test]
async fn delete_reports_a_missing_row_on_the_delete() {
    let (_, _, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Refused));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"ghost"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["detail"]["error"],
        "Invitation id does not exist in the database."
    );
}

#[tokio::test]
async fn delete_reports_a_store_failure() {
    let (store, _, router) = setup(Vec::new(), FakeScope::new(ScopeRights::Refused));
    store.rows.lock().expect("the store lock").delete_error = Some(DbError::NoDbConnected);
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, INTERNAL_SERVER_ERROR_BODY);
}

#[tokio::test]
async fn delete_requires_the_database() {
    let router = router_for(InvitationRouteState::default());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(parse(&text)["detail"]["error"], DB_NOT_CONNECTED);
}

#[tokio::test]
async fn delete_reports_a_database_it_cannot_reach() {
    let scope: Arc<dyn InvitationScope> = Arc::new(open_scope());
    let store = PostgresInvitations {
        store: PostgresStore { pool: None },
    };
    let router = router_for(InvitationRouteState {
        invitations: Some(InvitationDatabase {
            invitations: Arc::new(store),
            scope,
        }),
        queue: QueueRouteState::default(),
    });
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, INTERNAL_SERVER_ERROR_BODY);
}

#[tokio::test]
async fn delete_by_a_proxy_admin_skips_the_ownership_check() {
    let (store, _, router) = setup(
        vec![record("inv-1", "alice", "someone-else")],
        FakeScope::new(ScopeRights::Privileged),
    );
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/invitation/delete",
        r#"{"invitation_id":"inv-1"}"#,
        Some(platform_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store.rows.lock().expect("the store lock").deleted,
        vec!["inv-1".to_owned()]
    );
}
