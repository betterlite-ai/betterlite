#![cfg(test)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};
use secrecy::SecretString;
use serde::Serialize;
use tower::ServiceExt as _;

use betterlite_auth::auth_identity::errors::AuthError;

use super::{
    AdmissionLease, AuthBackend, AuthContext, BudgetAdmission, RequestScope, stage_middleware,
};
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
    Arc::new(AppState {
        master_key: Some(SecretString::from(master_key.to_owned())),
        auth_backends: vec![AuthBackend::MasterKey],
    })
}

#[derive(Serialize)]
struct ProbeBody {
    subject: String,
}

async fn probe(Extension(context): Extension<AuthContext>) -> Json<ProbeBody> {
    Json(ProbeBody {
        subject: context.principal.subject,
    })
}

fn protected_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/probe", get(probe))
        .layer(middleware::from_fn_with_state(state, stage_middleware))
}

fn request_with_key(key: Option<&str>) -> Request<Body> {
    let unauthenticated = Request::builder().uri("/probe");
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

    let missing = protected_router(Arc::clone(&state))
        .oneshot(request_with_key(None))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let wrong = protected_router(Arc::clone(&state))
        .oneshot(request_with_key(Some("wrong-key")))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(wrong.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("no_db_connection"));

    let correct = protected_router(state)
        .oneshot(request_with_key(Some("secret-master-key")))
        .await
        .unwrap();
    assert_eq!(correct.status(), StatusCode::OK);
}

#[tokio::test]
async fn failure_stops_downstream() {
    let state = test_state("secret-master-key");
    let response = protected_router(state)
        .oneshot(request_with_key(Some("wrong-key")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("master_key"));
}

#[tokio::test]
async fn success_forwards_auth_context() {
    let state = test_state("secret-master-key");
    let response = protected_router(state)
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
    let response = protected_router(state)
        .oneshot(request_with_key(Some("presented-leaked-secret")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("presented-leaked-secret"));
}
