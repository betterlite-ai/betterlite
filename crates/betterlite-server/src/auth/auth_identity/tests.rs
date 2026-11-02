#![cfg(test)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use secrecy::SecretString;
use tower::ServiceExt as _;

use betterlite_auth::auth_identity::errors::AuthError;

use super::{AdmissionLease, AuthBackend, AuthContext, BudgetAdmission, RequestScope};
use crate::app::build;
use crate::state::AppState;

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
    let context = super::master_key_context();
    let scope = RequestScope {
        route: "/v1/auth/context".to_owned(),
    };
    assert!(matches!(
        AlwaysAdmits.admit(&context, &scope),
        Ok(AdmissionLease)
    ));
}

fn test_state(master_key: &str) -> Arc<AppState> {
    AppState::new(Some(SecretString::from(master_key.to_owned())))
}

fn request_with_key(key: Option<&str>) -> Request<Body> {
    let unauthenticated = Request::builder().uri("/v1/auth/context");
    let authenticated = match key {
        Some(value) => unauthenticated.header(header::AUTHORIZATION, format!("Bearer {value}")),
        None => unauthenticated,
    };
    authenticated.body(Body::empty()).unwrap()
}

#[test]
fn stage_is_registered() {
    let state = test_state("secret-master-key");
    assert!(state.auth_backends.contains(&AuthBackend::MasterKey));
}

#[tokio::test]
async fn default_stages_keep_master_key_behavior() {
    let state = test_state("secret-master-key");

    let missing = build(Arc::clone(&state))
        .oneshot(request_with_key(None))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = build(Arc::clone(&state))
        .oneshot(request_with_key(Some("wrong-key")))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let correct = build(state)
        .oneshot(request_with_key(Some("secret-master-key")))
        .await
        .unwrap();
    assert_eq!(correct.status(), StatusCode::OK);
}

#[tokio::test]
async fn failure_stops_downstream() {
    let state = test_state("secret-master-key");
    let response = build(state)
        .oneshot(request_with_key(Some("wrong-key")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("master_key"));
}

#[tokio::test]
async fn success_forwards_auth_context() {
    let state = test_state("secret-master-key");
    let response = build(state)
        .oneshot(request_with_key(Some("secret-master-key")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("master_key"));
}

#[tokio::test]
async fn resolver_error_redacts_presented_credentials() {
    let state = test_state("secret-master-key");
    let response = build(state)
        .oneshot(request_with_key(Some("presented-leaked-secret")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("presented-leaked-secret"));
}
