#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;

use super::support::{
    Events, FakeCache, FakeDatabase, Setup, World, admin, build, flags_blocked,
    flags_blocked_team_admins, flags_open, internal, viewer,
};
use super::{at, content_type, field, field_count, text};
use crate::api::vector_management_write::state::VectorManagementWriteState;
use crate::api::vector_management_write::{DELETE_PATH, NEW_PATH, UPDATE_PATH};
use crate::auth::auth_identity::AuthContext;

type WriteState = Arc<VectorManagementWriteState<FakeDatabase>>;

const CREATE_BODY: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai"}"#;
const SECOND_CREATE_BODY: &str = r#"{"vector_store_id":"vs_b","custom_llm_provider":"openai"}"#;
const DELETE_BODY: &str = r#"{"vector_store_id":"vs_a"}"#;
const UPDATE_BODY: &str = r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#;
const DISABLED: &str =
    "Access to vector_stores is disabled for your role. Contact your proxy admin.";

#[tokio::test]
async fn management_write_auth_and_feature_gate() {
    assert_switch_refuses_every_route().await;
    assert_admins_bypass_the_switch().await;
    assert_internal_user_is_served_by_default().await;
    assert_team_admin_flag_needs_verification().await;
    assert_unprocessable_bodies_never_reach_the_repository().await;
}

// the switch refuses all three routes before the handler looks at the database
async fn assert_switch_refuses_every_route() {
    let blocked = build(Setup {
        database: None,
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: flags_blocked(),
        events: Events::default(),
    });
    assert_disabled(&blocked, NEW_PATH, CREATE_BODY).await;
    assert_disabled(&blocked, DELETE_PATH, DELETE_BODY).await;
    assert_disabled(&blocked, UPDATE_PATH, UPDATE_BODY).await;
}

// the proxy admin and the view-only admin keep their access
async fn assert_admins_bypass_the_switch() {
    let admins = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: flags_blocked(),
        events: Events::default(),
    });
    assert_eq!(
        StatusCode::OK,
        status_of(&admins, &admin(), NEW_PATH, CREATE_BODY).await
    );
    assert_eq!(
        StatusCode::OK,
        status_of(&admins, &viewer(), NEW_PATH, SECOND_CREATE_BODY).await
    );
}

// both switches default to false, so an internal user is served
async fn assert_internal_user_is_served_by_default() {
    let open = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: flags_open(),
        events: Events::default(),
    });
    assert_eq!(
        StatusCode::OK,
        status_of(&open, &internal("u1", None), NEW_PATH, CREATE_BODY).await
    );
}

// the allow flag only exempts a caller B864 verifies as a team admin
async fn assert_team_admin_flag_needs_verification() {
    let privileged = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: true,
        settings: flags_blocked_team_admins(),
        events: Events::default(),
    });
    assert_eq!(
        StatusCode::OK,
        status_of(&privileged, &internal("u1", None), NEW_PATH, CREATE_BODY).await
    );
    let unverified = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: flags_blocked_team_admins(),
        events: Events::default(),
    });
    assert_eq!(
        StatusCode::FORBIDDEN,
        status_of(&unverified, &internal("u1", None), NEW_PATH, CREATE_BODY).await
    );
}

// an id the framework validates away never reaches the switch or the repository
async fn assert_unprocessable_bodies_never_reach_the_repository() {
    let events = Events::default();
    let open = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: None,
        registry: Some(Vec::new()),
        world: World::default(),
        privileges: false,
        settings: flags_open(),
        events: events.clone(),
    });
    assert_unprocessable(&open, DELETE_PATH).await;
    assert_unprocessable(&open, UPDATE_PATH).await;
    assert!(events.entries().is_empty());
}

async fn status_of(
    state: &WriteState,
    caller: &AuthContext,
    path: &str,
    payload: &str,
) -> StatusCode {
    let (status, _, _) = super::support::call(state, caller, path, payload).await;
    status
}

async fn assert_disabled(state: &WriteState, path: &str, payload: &str) {
    let (status, headers, body) =
        super::support::call(state, &internal("u1", None), path, payload).await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    assert_eq!(Some("application/json"), content_type(&headers));
    assert_eq!(1, field_count(&body));
    let detail = at(&body, "detail");
    assert_eq!(1, field_count(detail));
    assert_eq!(DISABLED, text(at(detail, "error")));
    assert_eq!(None, field(&body, "type"));
    assert_eq!(None, field(&body, "param"));
    assert_eq!(None, field(&body, "code"));
}

async fn assert_unprocessable(state: &WriteState, path: &str) {
    let (status, headers, body) =
        super::support::call(state, &internal("u1", None), path, "{}").await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert_eq!(Some("application/json"), content_type(&headers));
    assert!(matches!(field(&body, "detail"), Some(Json::List(_))));
}
