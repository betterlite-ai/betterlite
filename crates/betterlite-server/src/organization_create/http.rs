use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::error_body::{AuthenticationFailure, authentication_error};
use crate::organization_create::creation::{OrganizationCreateState, create_organization};
use crate::organization_create::records::NewOrganizationRequest;

const NEW_ORGANIZATION_PATH: &str = "/organization/new";

pub(crate) fn mount(state: Arc<OrganizationCreateState>) -> Router {
    Router::new()
        .route(NEW_ORGANIZATION_PATH, post(create))
        .with_state(state)
}

#[derive(Serialize)]
struct ValidationBody {
    detail: String,
}

async fn create(
    State(state): State<Arc<OrganizationCreateState>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return authentication_error(&AuthenticationFailure::Missing);
    };
    let request = match serde_json::from_slice::<NewOrganizationRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ValidationBody {
                    detail: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    match create_organization(&state, &context.principal, request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => error.into_response(),
    }
}
