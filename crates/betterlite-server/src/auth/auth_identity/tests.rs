#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use secrecy::SecretString;
use tower::ServiceExt as _;

use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_config::sections::auth_route_patterns::Settings;

use crate::auth::auth_route_patterns::AuthSettings;
use crate::routes;

use super::{
    AdmissionLease, AuthBackend, AuthContext, BudgetAdmission, INSTALLED_STAGES, RequestScope,
    StageInput, dispatch,
};

struct AlwaysAdmits;

impl BudgetAdmission for AlwaysAdmits {
    fn admit(
        &self,
        _context: &AuthContext,
        _scope: &RequestScope,
    ) -> Result<AdmissionLease, AuthError> {
        Ok(AdmissionLease)
    }
}

#[test]
fn budget_admission_port_is_implementable() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, &master_key) else {
        panic!("master key dispatch should succeed");
    };
    let scope = RequestScope {
        route: "/v1/models".to_owned(),
    };
    assert!(matches!(
        AlwaysAdmits.admit(&context, &scope),
        Ok(AdmissionLease)
    ));
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::MasterKey));
}

fn protected_server(master_key: &str) -> Router {
    protected_server_with_chat(master_key, Router::new())
}

fn protected_server_with_chat(master_key: &str, chat: Router) -> Router {
    routes::router(
        AuthSettings {
            master_key: Some(SecretString::from(master_key.to_owned())),
            routes: Settings::default(),
        },
        Arc::new(AtomicBool::new(false)),
        &[],
        chat,
        Router::new(),
    )
}

async fn echo_principal_subject(Extension(context): Extension<AuthContext>) -> String {
    context.principal.subject
}

fn request_to(path: &str, key: Option<&str>) -> Request<Body> {
    let unauthenticated = Request::builder().uri(path);
    let authenticated = match key {
        Some(value) => unauthenticated.header(header::AUTHORIZATION, format!("Bearer {value}")),
        None => unauthenticated,
    };
    authenticated.body(Body::empty()).unwrap()
}

fn request_with_key(key: Option<&str>) -> Request<Body> {
    request_to("/v1/models", key)
}

#[tokio::test]
async fn default_stages_keep_master_key_behavior() {
    let missing = protected_server("secret-master-key")
        .oneshot(request_with_key(None))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let wrong = protected_server("secret-master-key")
        .oneshot(request_with_key(Some("wrong-key")))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(wrong.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("no_db_connection"));

    let correct = protected_server("secret-master-key")
        .oneshot(request_with_key(Some("secret-master-key")))
        .await
        .unwrap();
    assert_eq!(correct.status(), StatusCode::OK);
}

#[tokio::test]
async fn failure_stops_downstream() {
    let response = protected_server("secret-master-key")
        .oneshot(request_with_key(Some("wrong-key")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("\"object\":\"list\""));
    assert!(!text.contains("master_key"));
}

#[tokio::test]
async fn success_forwards_auth_context() {
    let chat = Router::new().route("/v1/chat/completions", get(echo_principal_subject));
    let response = protected_server_with_chat("secret-master-key", chat)
        .oneshot(request_to(
            "/v1/chat/completions",
            Some("secret-master-key"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("master_key"));
}

#[tokio::test]
async fn resolver_error_redacts_presented_credentials() {
    let response = protected_server("secret-master-key")
        .oneshot(request_with_key(Some("presented-leaked-secret")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("presented-leaked-secret"));
}
