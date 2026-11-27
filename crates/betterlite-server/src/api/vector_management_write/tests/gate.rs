#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;

use super::support::{
    Events, FakeCache, FakeDatabase, Setup, admin, build, flags_blocked, flags_blocked_team_admins,
    flags_open, internal, viewer,
};
use super::{at, content_type, field, field_count, text};
use crate::api::vector_management_write::state::VectorManagementWriteState;
use crate::api::vector_management_write::{DELETE_PATH, NEW_PATH, UPDATE_PATH};

const CREATE_BODY: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai"}"#;
const DELETE_BODY: &str = r#"{"vector_store_id":"vs_a"}"#;
const UPDATE_BODY: &str = r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#;
const DISABLED: &str =
    "Access to vector_stores is disabled for your role. Contact your proxy admin.";

#[tokio::test]
async fn management_write_auth_and_feature_gate() {
    // the switch refuses all three routes before the handler looks at the database
    let mut setup = Setup::default();
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.settings = flags_blocked();
    let blocked = build(setup);
    assert_disabled(&blocked, NEW_PATH, CREATE_BODY).await;
    assert_disabled(&blocked, DELETE_PATH, DELETE_BODY).await;
    assert_disabled(&blocked, UPDATE_PATH, UPDATE_BODY).await;
    // the proxy admin and the view-only admin keep their access
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.settings = flags_blocked();
    let admins = build(setup);
    let (status, _headers, _body) =
        super::support::call(&admins, &admin(), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    let (status, _headers, _body) =
        super::support::call(&admins, &viewer(), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    // both switches default to false, so an internal user is served
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.settings = flags_open();
    let open = build(setup);
    let (status, _headers, _body) =
        super::support::call(&open, &internal("u1", None), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    // the allow flag only exempts a caller B864 verifies as a team admin
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.privileges = true;
    setup.settings = flags_blocked_team_admins();
    let privileged = build(setup);
    let (status, _headers, _body) =
        super::support::call(&privileged, &internal("u1", None), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.cache = Some(FakeCache::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.privileges = false;
    setup.settings = flags_blocked_team_admins();
    let unverified = build(setup);
    let (status, _headers, _body) =
        super::support::call(&unverified, &internal("u1", None), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    // an id the framework validates away never reaches the switch or the repository
    let events = Events::default();
    let mut setup = Setup::default();
    setup.database = Some(FakeDatabase::new(Vec::new()));
    setup.registry = Some(Vec::new());
    setup.events = events.clone();
    let open = build(setup);
    assert_unprocessable(&open, DELETE_PATH).await;
    assert_unprocessable(&open, UPDATE_PATH).await;
    assert!(events.entries().is_empty());
}

async fn assert_disabled(
    state: &Arc<VectorManagementWriteState<FakeDatabase>>,
    path: &str,
    payload: &str,
) {
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

async fn assert_unprocessable(state: &Arc<VectorManagementWriteState<FakeDatabase>>, path: &str) {
    let (status, headers, body) =
        super::support::call(state, &internal("u1", None), path, "{}").await;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert_eq!(Some("application/json"), content_type(&headers));
    assert!(matches!(field(&body, "detail"), Some(Json::List(_))));
}
