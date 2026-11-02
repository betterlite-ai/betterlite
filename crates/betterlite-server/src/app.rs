use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::middleware;
use axum::routing::get;
use serde::Serialize;

use crate::auth::auth_identity::{AuthContext, stage_middleware};
use crate::state::AppState;

pub fn build(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/auth/context", get(auth_context))
        .layer(middleware::from_fn_with_state(state, stage_middleware))
}

async fn auth_context(Extension(context): Extension<AuthContext>) -> Json<AuthContextBody> {
    Json(AuthContextBody {
        subject: context.principal.subject,
        principal_type: context.principal.principal_type.to_string(),
    })
}

#[derive(Serialize)]
struct AuthContextBody {
    subject: String,
    principal_type: String,
}
