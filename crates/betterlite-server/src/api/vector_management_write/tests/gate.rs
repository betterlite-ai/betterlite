#![cfg(test)]

use axum::http::StatusCode;
use betterlite_models::decisions::Json;

use super::support::{
    Events, FakeCache, FakeDatabase, Setup, admin, build, flags_blocked, flags_blocked_team_admins,
    flags_open, internal, viewer,
};
use super::{at, content_type, field, field_count, text};
use crate::api::vector_management_write::{DELETE_PATH, NEW_PATH, UPDATE_PATH};

const CREATE_BODY: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai"}"#;
const DELETE_BODY: &str = r#"{"vector_store_id":"vs_a"}"#;
const UPDATE_BODY: &str = r#"{"vector_store_id":"vs_a","vector_store_description":"x"}"#;
const DISABLED: &str =
    "Access to vector_stores is disabled for your role. Contact your proxy admin.";

#[tokio::test]
async fn management_write_auth_and_feature_gate() {
    // the switch refuses all three routes before the handler looks at the database
    let blocked = build(Setup {
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        settings: flags_blocked(),
        ..Setup::default()
    });
    for (path, payload) in [
        (NEW_PATH, CREATE_BODY),
        (DELETE_PATH, DELETE_BODY),
        (UPDATE_PATH, UPDATE_BODY),
    ] {
        let (status, headers, body) =
            super::support::call(&blocked, &internal("u1", None), path, payload).await;
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
    // the proxy admin and the view-only admin keep their access
    let admins = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        settings: flags_blocked(),
        ..Setup::default()
    });
    for caller in [admin(), viewer()] {
        let (status, _headers, _body) =
            super::support::call(&admins, &caller, NEW_PATH, CREATE_BODY).await;
        assert_eq!(StatusCode::OK, status);
    }
    // both switches default to false, so an internal user is served
    let open = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        settings: flags_open(),
        ..Setup::default()
    });
    let (status, _headers, _body) =
        super::support::call(&open, &internal("u1", None), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    // the allow flag only exempts a caller B864 verifies as a team admin
    let privileged = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        privileges: true,
        settings: flags_blocked_team_admins(),
        ..Setup::default()
    });
    let (status, _headers, _body) =
        super::support::call(&privileged, &internal("u1", None), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::OK, status);
    let unverified = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        cache: Some(FakeCache::new(Vec::new())),
        registry: Some(Vec::new()),
        privileges: false,
        settings: flags_blocked_team_admins(),
        ..Setup::default()
    });
    let (status, _headers, _body) =
        super::support::call(&unverified, &internal("u1", None), NEW_PATH, CREATE_BODY).await;
    assert_eq!(StatusCode::FORBIDDEN, status);
    // an id the framework validates away never reaches the switch or the repository
    let events = Events::default();
    let open = build(Setup {
        database: Some(FakeDatabase::new(Vec::new())),
        registry: Some(Vec::new()),
        events: events.clone(),
        ..Setup::default()
    });
    for path in [DELETE_PATH, UPDATE_PATH] {
        let (status, headers, body) =
            super::support::call(&open, &internal("u1", None), path, "{}").await;
        assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
        assert_eq!(Some("application/json"), content_type(&headers));
        assert!(matches!(field(&body, "detail"), Some(Json::List(_))));
    }
    assert!(events.entries().is_empty());
}
