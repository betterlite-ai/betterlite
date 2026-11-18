use std::error::Error;
use std::fmt::Display;

use thiserror::Error;

use super::text::TextExtractionError;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::semantic_guard::semantic_guard_route_loading::builder::RouterBuildError;
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;

pub const NO_ROUTES_MESSAGE: &str =
    "SemanticGuardrail: no routes configured. Provide route_templates or custom_routes.";
pub const INTERNAL_STATUS: u16 = 500;

#[derive(Debug, Error)]
pub enum SemanticGuardSetupError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error("SemanticGuardrail: no routes configured. Provide route_templates or custom_routes.")]
    NoRoutes,
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error(transparent)]
    Load(#[from] RouteLoadError),
    #[error(transparent)]
    Build(#[from] RouterBuildError<E>),
}

// the hook raises these out of the request, so they answer as an internal error
#[derive(Debug, Error)]
pub enum SemanticGuardHookError {
    #[error("the semantic guardrail request messages could not be read: {source}")]
    Messages {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("the semantic guardrail text could not be extracted: {source}")]
    Text {
        #[source]
        source: TextExtractionError,
    },
}

impl From<SemanticGuardHookError> for GuardrailFailure {
    fn from(error: SemanticGuardHookError) -> Self {
        Self::Http {
            status: INTERNAL_STATUS,
            detail: error.to_string(),
        }
    }
}

// the router's error type belongs to the builder, so the scoring failure is built where it is raised
pub fn scoring_failure<E>(source: &E) -> GuardrailFailure
where
    E: Display + ?Sized,
{
    GuardrailFailure::Http {
        status: INTERNAL_STATUS,
        detail: format!("the semantic guardrail text could not be scored: {source}"),
    }
}
