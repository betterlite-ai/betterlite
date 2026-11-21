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

use super::{AuthRequestPolicyStageInput, run_stage};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

const MODEL_ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";
const DENIED_IP: &str = "203.0.113.9";

fn config_with(settings: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "general_settings:\n  master_key: {MASTER_KEY}\n{settings}"
    ))
    .expect("the test config loads")
}

fn config() -> Config {
    config_with("  allowed_ips: [10.0.0.1]\n  use_x_forwarded_for: true\n")
}

fn master_context() -> AuthContext {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, &master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn stage_input(settings: &Config) -> AuthRequestPolicyStageInput {
    AuthRequestPolicyStageInput {
        context: master_context(),
        content_length: Some("2".to_owned()),
        forwarded_for: Some(DENIED_IP.to_owned()),
        client_host: None,
        route: MODEL_ROUTE.to_owned(),
        content_type: Some("application/json".to_owned()),
        body: Bytes::from_static(b"{}"),
        slot: Some(state_for(settings).auth_request_policy.clone()),
    }
}

#[test]
fn stage_is_registered() {
    let policy = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthRequestPolicy)
        .expect("the policy stage is installed");
    let safety = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::RequestBodySafety)
        .expect("the safety stage is installed");
    assert!(
        policy < safety,
        "the ordered checks run before the safety stage"
    );

    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let input = StageInput::AuthRequestPolicy(Box::new(stage_input(&config())));
    let Err(StageFailure::AuthRequestPolicy(exception)) =
        dispatch(AuthBackend::AuthRequestPolicy, &input, &master_key)
    else {
        panic!("the production stage refuses a client outside allowed_ips");
    };
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "403");
    assert!(
        exception
            .message
            .contains("Access forbidden: IP address 203.0.113.9 not allowed."),
        "{}",
        exception.message
    );
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>, body: Bytes) -> String {
    reached.store(true, Ordering::Release);
    String::from_utf8(body.to_vec()).unwrap_or_default()
}

fn application(config: &Config, reached: &Arc<AtomicBool>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::clone(reached)))
}

fn request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {MASTER_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", DENIED_IP)
        .body(Body::from(r#"{"model":"gpt-4o","messages":[]}"#))
        .expect("the request builds")
}

#[tokio::test]
async fn failure_stops_downstream() {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(&config(), &reached)
        .oneshot(request())
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(bytes.to_vec()).expect("the response body is UTF-8");
    assert!(text.contains("\"type\":\"auth_error\""), "{text}");
    assert!(text.contains("\"param\":\"None\""), "{text}");
    assert!(text.contains("\"code\":\"403\""), "{text}");
    assert!(text.contains(DENIED_IP), "{text}");
    assert!(!reached.load(Ordering::Acquire));
}

#[tokio::test]
async fn allowed_route_reaches_downstream() {
    // the stage reads the request's own route and the configured list admits it; the body is not JSON
    // so no later stage resolves a model
    let reached = Arc::new(AtomicBool::new(false));
    let allowed = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {MASTER_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("x"))
        .expect("the request builds");
    let response = application(&config_with("  allowed_routes: [/v1/files]\n"), &reached)
        .oneshot(allowed)
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(reached.load(Ordering::Acquire));
}

#[test]
fn allowed_client_reaches_the_next_stage() {
    let mut input = stage_input(&config());
    input.forwarded_for = Some("10.0.0.1".to_owned());
    let context = run_stage(&input).expect("an allowed client passes");
    assert_eq!(context.principal.subject, "master_key");
}
