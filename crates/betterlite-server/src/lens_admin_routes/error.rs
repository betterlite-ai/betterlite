use std::error::Error as StdError;

use axum::Json;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::lens_repository::repository::LensRepositoryError;
use betterlite_db::lens_repository::update::LensUpdateError;
use betterlite_lens::models::settings::Scope;
use betterlite_lens::sources::source_reader::SourceError;
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::TransitionError;
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::{LensAccessError, failure_text};
use crate::lens_access::scope::{caller_key, user_scope};
use crate::lens_billing::key_validation::KeyValidationError;
use crate::tracing_routes::router::{DetailBody, unauthenticated};

use super::body::{BodyFailure, QueryFailure};

// the failures a lens transform raises; no client input reaches them, so every one is a 500
#[derive(Debug, Error)]
pub(crate) enum LensMutationError {
    #[error("the lens revision would overflow")]
    Revision,
    #[error("the lens deadline leaves the supported date range")]
    Deadline,
    #[error(transparent)]
    Transition(#[from] TransitionError),
}

#[derive(Debug, Error)]
pub(crate) enum LensRouteError<E>
where
    E: StdError + 'static,
{
    #[error("the caller is not authenticated")]
    Unauthenticated,
    #[error(transparent)]
    Access(#[from] LensAccessError),
    #[error(transparent)]
    Key(#[from] KeyValidationError),
    #[error(transparent)]
    Repository(#[from] LensRepositoryError),
    #[error(transparent)]
    Mutation(#[from] LensMutationError),
    #[error("the lens storage query failed")]
    Source(#[from] SourceError<E>),
    #[error(transparent)]
    Body(#[from] BodyFailure),
    #[error(transparent)]
    Query(#[from] QueryFailure),
}

pub(crate) fn update_failure<E>(error: LensUpdateError<LensMutationError>) -> LensRouteError<E>
where
    E: StdError + 'static,
{
    match error {
        LensUpdateError::Repository(source) => LensRouteError::Repository(source),
        LensUpdateError::Transform(source) => LensRouteError::Mutation(source),
    }
}

// the resolved key and its scope: the write flag refuses a non-admin before the handler does anything else
pub(crate) fn route_key_scope<Store>(
    caller: Option<Extension<AuthContext>>,
    write: bool,
) -> Result<(UserAPIKeyAuth, Scope), LensRouteError<<Store as Storage>::Error>>
where
    Store: Storage,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let key = caller_key(&context.principal)?;
    let scope = user_scope(&key, write)?;
    Ok((key, scope))
}

impl<E> IntoResponse for LensRouteError<E>
where
    E: StdError + Send + Sync + 'static,
{
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            Self::Unauthenticated => return unauthenticated(),
            Self::Body(error) => return error.into_response(),
            Self::Query(error) => return error.into_response(),
            Self::Access(error) => (error.status(), failure_text(&error)),
            Self::Key(error) => (
                StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                error.detail(),
            ),
            // a failure that is not one of LiteLLM's refusals: a 500 that names the whole cause chain
            Self::Repository(error) => (StatusCode::INTERNAL_SERVER_ERROR, failure_text(&error)),
            Self::Mutation(error) => (StatusCode::INTERNAL_SERVER_ERROR, failure_text(&error)),
            Self::Source(error) => (StatusCode::INTERNAL_SERVER_ERROR, failure_text(&error)),
        };
        (status, Json(DetailBody { detail })).into_response()
    }
}
