use std::error::Error as StdError;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::lens_repository::repository::LensRepositoryError;
use betterlite_db::lens_repository::update::LensUpdateError;
use betterlite_lens::sources::source_reader::SourceError;
use betterlite_lens::state::transitions::TransitionError;
use thiserror::Error;

use crate::lens_access::error::{LensAccessError, failure_text};
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

impl<E> IntoResponse for LensRouteError<E>
where
    E: StdError + Send + Sync + 'static,
{
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Body(error) => error.into_response(),
            Self::Query(error) => error.into_response(),
            Self::Access(error) => {
                let status = error.status();
                let detail = error.detail();
                (status, Json(DetailBody { detail })).into_response()
            }
            Self::Key(error) => {
                let status = StatusCode::from_u16(error.status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let detail = error.detail();
                (status, Json(DetailBody { detail })).into_response()
            }
            Self::Repository(error) => {
                let detail = failure_text(&error);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(DetailBody { detail }),
                )
                    .into_response()
            }
            Self::Mutation(error) => {
                let detail = failure_text(&error);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(DetailBody { detail }),
                )
                    .into_response()
            }
            Self::Source(error) => {
                let detail = failure_text(&error);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(DetailBody { detail }),
                )
                    .into_response()
            }
        }
    }
}
