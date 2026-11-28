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
use betterlite_config::sections::auth_user_lookup::RbacRole;
use betterlite_config::types::Config;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{JwtPermissionRule, JwtPermissionStageInput, request_model, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

const CHAT_ROUTE: &str = "/v1/chat/completions";
const USER_ROUTE: &str = "/user/new";
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

// enforce_rbac and the role mapping both come from general_settings in this config
fn role_config() -> Config {
    let source = format!(
        "general_settings:\n  master_key: {MASTER_KEY}\n  litellm_jwtauth:\n    enforce_rbac: true\n  \
         role_permissions:\n    - role: internal_user\n      models: ['gpt-4o']\n      routes: ['/user/new']\n"
    );
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .unwrap()
}

fn internal_rule() -> JwtPermissionRule {
    JwtPermissionRule {
        role: Some(RbacRole::InternalUser),
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
async fn run_request(
    config: &Config,
    rule: JwtPermissionRule,
    route: &str,
    body: &'static str,
) -> (StatusCode, String, bool) {
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(route, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = Request::builder()
        .method("POST")
        .uri(route)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
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

fn empty_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap()
}

#[test]
fn missing_context_is_unauthenticated() {
    assert!(matches!(
        stage_input(&empty_request(CHAT_ROUTE), None, None),
        Err(StageFailure::Authentication(_))
    ));
}

// a master key or virtual key request carries no JWT claims, so the stage must leave it alone
#[test]
fn no_rule_passes_the_context_through() {
    let context = master_context();
    let Ok(StageInput::JwtPermissions(stage)) =
        stage_input(&empty_request(CHAT_ROUTE), Some(&context), None)
    else {
        panic!("the stage builds its input without a rule");
    };
    let master = master_key();
    let Ok(forwarded) = dispatch(
        AuthBackend::JwtPermissions,
        &StageInput::JwtPermissions(stage),
        &master,
    ) else {
        panic!("an absent rule leaves the stage inert");
    };
    assert_eq!(forwarded.principal.subject, "master_key");
}

// request_data.get("model"): only the declared string field is a model
#[test]
fn request_model_reads_only_a_string() {
    let request = empty_request(CHAT_ROUTE);
    let text = Bytes::from_static(br#"{"model":"gpt-4o"}"#);
    assert_eq!(
        request_model(&request, Some(&text)),
        Some("gpt-4o".to_owned())
    );
    let number = Bytes::from_static(br#"{"model":5}"#);
    assert!(request_model(&request, Some(&number)).is_none());
    let absent = Bytes::from_static(br#"{}"#);
    assert!(request_model(&request, Some(&absent)).is_none());
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
    let (status, body, reached) = run_request(
        &configuration,
        admin_rule(),
        CHAT_ROUTE,
        r#"{"model":"gpt-4o"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
}

// the configured enforce_rbac, role mapping, request model and route all reach the decision
#[tokio::test]
async fn role_permissions_from_config() {
    let configuration = role_config();
    let (status, body, reached) = run_request(
        &configuration,
        internal_rule(),
        USER_ROUTE,
        r#"{"model":"gpt-4o"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(reached);
    let (status, body, reached) = run_request(
        &configuration,
        internal_rule(),
        USER_ROUTE,
        r#"{"model":"gpt-4o-mini"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("gpt-4o-mini"), "{body}");
    assert!(!reached);
}
