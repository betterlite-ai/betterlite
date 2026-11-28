#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{JwtPermissionRule, JwtPermissionStageInput, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

const CHAT_ROUTE: &str = "/v1/chat/completions";
const MASTER_KEY: &str = "secret-master-key";

fn loaded() -> Config {
    let source = format!("general_settings:\n  master_key: {MASTER_KEY}\n");
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .unwrap()
}

fn master_key() -> SecretString {
    SecretString::from(MASTER_KEY.to_owned())
}

fn master_context() -> AuthContext {
    let key = master_key();
    let Ok(context) = dispatch(
        AuthBackend::MasterKey,
        &StageInput::MasterKey(Some(key.clone())),
        &key,
    ) else {
        panic!("the master key stage authenticates");
    };
    context
}

// an admin-scope token that must still meet the configured admin_allowed_routes
fn admin_rule() -> JwtPermissionRule {
    JwtPermissionRule {
        scopes: vec!["litellm_proxy_admin".to_owned()],
        ..JwtPermissionRule::default()
    }
}

fn stage_from(rule: JwtPermissionRule, uri: &str, body: &'static str) -> JwtPermissionStageInput {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    request.extensions_mut().insert(Arc::new(rule));
    let context = master_context();
    let bytes = Bytes::from_static(body.as_bytes());
    let Ok(StageInput::JwtPermissions(stage)) = stage_input(&request, Some(&context), Some(&bytes))
    else {
        panic!("the JWT permission stage builds its own input");
    };
    *stage
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

// the real router with its auth chain, a handler that records whether it ran, and the rule the auth service inserts
async fn run_request(config: &Config, rule: JwtPermissionRule) -> (StatusCode, String, bool) {
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(CHAT_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = Request::builder()
        .method("POST")
        .uri(CHAT_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"model":"gpt-4o"}"#))
        .unwrap();
    request.extensions_mut().insert(Arc::clone(&reached));
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let answer = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        String::from_utf8(answer.to_vec()).unwrap(),
        reached.load(Ordering::Acquire),
    )
}

#[test]
fn stage_is_registered() {
    // LiteLLM runs the permission checks inside authorize_jwt, after the request body is readable
    let position = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::JwtPermissions);
    let after_safety = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::RequestBodySafety)
        .map(|index| index.saturating_add(1));
    assert_eq!(position, after_safety);
    let master = master_key();
    let denied = stage_from(admin_rule(), CHAT_ROUTE, r#"{"model":"gpt-4o"}"#);
    let Err(StageFailure::JwtPermissions(exception)) = dispatch(
        AuthBackend::JwtPermissions,
        &StageInput::JwtPermissions(Box::new(denied)),
        &master,
    ) else {
        panic!("an admin token is limited to the admin routes");
    };
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
    let wrong_stage = StageInput::MasterKey(Some(master.clone()));
    assert!(matches!(
        dispatch(AuthBackend::JwtPermissions, &wrong_stage, &master),
        Err(StageFailure::Authentication(_))
    ));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let configuration = loaded();
    let (status, body, reached) = run_request(&configuration, admin_rule()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
}
