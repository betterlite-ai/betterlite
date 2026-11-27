#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::identity_store::ports::Ports;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use tower::ServiceExt as _;

use super::lookup_stage_input;
use crate::auth::auth_identity::{AuthBackend, AuthContext, INSTALLED_STAGES, StageInput};
use crate::routes;
use crate::routes::tests::state_for;
use crate::state::ServerState;

mod support;
use support::{KEY_ROW, ports, reader, view_from_text};

const CHAT_ROUTE: &str = "/v1/chat/completions";
const PRESENTED: &str = "sk-virtual-key";
const MASTER_KEY: &str = "secret-master-key";

fn config() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

async fn mark_called(
    Extension(reached): Extension<Arc<AtomicBool>>,
    Extension(context): Extension<AuthContext>,
) -> String {
    reached.store(true, Ordering::Release);
    context.principal.subject
}

fn application(state: Arc<ServerState>) -> Router {
    routes::router(
        state,
        Arc::new(AtomicBool::new(false)),
        Router::new().route(CHAT_ROUTE, post(mark_called)),
        Router::new(),
    )
}

fn request(presented: &str) -> Request<Body> {
    Request::post(CHAT_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {presented}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"model":"gpt-4o","messages":[]}"#))
        .expect("the request builds")
}

fn staged_request(state: &Arc<ServerState>) -> Request<Body> {
    let mut request = request(PRESENTED);
    request.extensions_mut().insert(Arc::clone(state));
    request
}

async fn run(state: Arc<ServerState>, presented: &str) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = request(presented);
    request.extensions_mut().insert(Arc::clone(&reached));
    let response = application(state)
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
}

#[test]
fn stage_is_registered() {
    assert_eq!(INSTALLED_STAGES.first(), Some(&AuthBackend::AuthKeyLookup));
    let state = state_for(&config());
    assert!(state.identity_store.store().is_none());
    let uninstalled = staged_request(&state);
    assert!(matches!(
        lookup_stage_input(&uninstalled),
        StageInput::AuthKeyLookup(_)
    ));
    let (port_set, _writer) = ports(None);
    state
        .identity_store
        .install(port_set)
        .expect("the ports install");
    let installed = staged_request(&state);
    let StageInput::AuthIdentity(identity_stage) = lookup_stage_input(&installed) else {
        panic!("the installed store takes the auth key lookup stage");
    };
    let text = format!("{identity_stage:?}");
    assert!(text.contains("presented: true"));
    assert!(!text.contains(PRESENTED));
}

#[test]
fn required_ports_missing() {
    let state = state_for(&config());
    let error = state.identity_store.install(Ports::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the identity store is missing required ports: cache, permissions, writer"
    );
    assert!(state.identity_store.store().is_none());
}

#[tokio::test]
async fn failure_stops_downstream() {
    missing_key_refused().await;
    empty_key_is_a_missing_key().await;
    no_database_refused().await;
    valid_key_reaches_handler().await;
    master_key_wins_before_the_store().await;
}

// the empty key becomes AuthenticationFailure::Missing, and the shared auth stage answers it with its own 500 (B757's split) where LiteLLM returns a 401 auth_error
async fn empty_key_is_a_missing_key() {
    let state = state_for(&config());
    let (port_set, writer) = ports(Some(reader(None)));
    state
        .identity_store
        .install(port_set)
        .expect("the ports install");
    let (status, body, reached) = run(state, "").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "Internal Server Error");
    assert!(!reached);
    assert!(writer.writes().is_empty());
}

async fn missing_key_refused() {
    let state = state_for(&config());
    let (port_set, writer) = ports(Some(reader(None)));
    state
        .identity_store
        .install(port_set)
        .expect("the ports install");
    let (status, body, reached) = run(state, PRESENTED).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.contains("\"type\":\"token_not_found_in_db\""),
        "{body}"
    );
    assert!(body.contains("\"param\":\"key\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
    assert!(writer.writes().is_empty());
}

async fn no_database_refused() {
    let state = state_for(&config());
    let (port_set, writer) = ports(None);
    state
        .identity_store
        .install(port_set)
        .expect("the ports install");
    let (status, body, reached) = run(state, PRESENTED).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(
        body.contains("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"),
        "{body}"
    );
    assert!(!reached);
    assert!(writer.writes().is_empty());
}

async fn valid_key_reaches_handler() {
    let state = state_for(&config());
    let (port_set, writer) = ports(Some(reader(Some(view_from_text(KEY_ROW)))));
    state
        .identity_store
        .install(port_set)
        .expect("the ports install");
    let (status, body, reached) = run(state, PRESENTED).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    assert_eq!(body, "u1");
    let recorded = writer.writes();
    assert_eq!(recorded.len(), 1);
    assert_ne!(recorded.first().map(String::as_str), Some(PRESENTED));
}

async fn master_key_wins_before_the_store() {
    let state = state_for(&config());
    let (port_set, writer) = ports(Some(reader(None)));
    state
        .identity_store
        .install(port_set)
        .expect("the ports install");
    let (status, body, reached) = run(state, MASTER_KEY).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    assert_eq!(body, "master_key");
    assert!(writer.writes().is_empty());
}
