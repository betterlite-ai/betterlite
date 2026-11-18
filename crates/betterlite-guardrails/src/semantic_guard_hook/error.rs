use std::error::Error;

use thiserror::Error;

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
    Routes(#[from] RouteLoadError),
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error(transparent)]
    Build(#[from] RouterBuildError<E>),
}

// LiteLLM's join raises on a dict part whose text is not a string, and on text it cannot print
#[derive(Debug, Error)]
pub enum TextExtractionError {
    #[error("the content part at index {index} carries a text that is not a string")]
    NonTextPart { index: usize },
    #[error("the content part at index {index} could not be written as text")]
    UnrepresentablePart { index: usize },
}

// the hooks raise these with each cause kept as its source, and the match action's failure riding along
#[derive(Debug, Error)]
pub enum SemanticGuardHookError<E>
where
    E: Error + Send + Sync + 'static,
{
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
    #[error("the semantic guardrail text could not be scored: {source}")]
    Scoring {
        #[source]
        source: E,
    },
    #[error(transparent)]
    Blocked(GuardrailFailure),
}

impl<E> From<SemanticGuardHookError<E>> for GuardrailFailure
where
    E: Error + Send + Sync + 'static,
{
    fn from(error: SemanticGuardHookError<E>) -> Self {
        match error {
            SemanticGuardHookError::Blocked(failure) => failure,
            other => Self::Http {
                status: INTERNAL_STATUS,
                detail: other.to_string(),
            },
        }
    }
}
