use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::resolved_key::credential::hash_token;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::state::AppState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthBackend {
    MasterKey,
}

#[derive(Clone, Debug)]
pub enum StageInput {
    MasterKey(Option<SecretString>),
}

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub principal: Principal,
}

#[derive(Clone, Debug)]
pub struct RequestScope {
    pub route: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionLease;

pub trait BudgetAdmission {
    fn admit(
        &self,
        context: &AuthContext,
        scope: &RequestScope,
    ) -> Result<AdmissionLease, AuthError>;
}

pub fn dispatch(
    backend: AuthBackend,
    input: &StageInput,
    master_key: Option<&SecretString>,
) -> Result<AuthContext, AuthError> {
    match backend {
        AuthBackend::MasterKey => master_key_stage(input, master_key),
    }
}

fn master_key_stage(
    input: &StageInput,
    master_key: Option<&SecretString>,
) -> Result<AuthContext, AuthError> {
    let StageInput::MasterKey(presented) = input;
    let Some(expected) = master_key else {
        return Ok(master_key_context());
    };
    let matched = presented.as_ref().is_some_and(|key| {
        bool::from(
            key.expose_secret()
                .as_bytes()
                .ct_eq(expected.expose_secret().as_bytes()),
        )
    });
    if matched {
        Ok(master_key_context())
    } else {
        Err(AuthError::KeyNotFound {
            hashed_token: hash_token(presented.as_ref().map_or("", SecretString::expose_secret)),
        })
    }
}

fn master_key_context() -> AuthContext {
    AuthContext {
        principal: Principal::new(
            PrincipalType::ServiceAccount,
            "master_key".to_owned(),
            AuthMethod::ApiKey,
        ),
    }
}

pub(crate) async fn stage_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let Some(master_key) = state.master_key.as_ref() else {
        return next.run(request).await;
    };
    let presented = super::extract_key(request.headers());
    for backend in &state.auth_backends {
        let input = StageInput::MasterKey(presented.clone());
        match dispatch(*backend, &input, Some(master_key)) {
            Ok(context) => {
                request.extensions_mut().insert(context);
            }
            Err(error) => return error_response(&error),
        }
    }
    next.run(request).await
}

fn error_response(error: &AuthError) -> Response {
    let Some(mapping) = error.http_mapping() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: ErrorObject {
                    message: error.to_string(),
                    error_type: WireField::Text(error.kind().to_owned()),
                    param: WireField::Null,
                    code: WireField::Null,
                },
            }),
        )
            .into_response();
    };
    let status = StatusCode::from_u16(mapping.status).unwrap_or(StatusCode::UNAUTHORIZED);
    (
        status,
        Json(ErrorResponse {
            error: ErrorObject {
                message: error.to_string(),
                error_type: WireField::Text(mapping.error_type.to_owned()),
                param: WireField::Text(mapping.param.to_owned()),
                code: WireField::Text(mapping.code.to_owned()),
            },
        }),
    )
        .into_response()
}

mod tests;
