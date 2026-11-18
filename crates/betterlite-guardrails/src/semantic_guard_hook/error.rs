use std::error::Error;

use thiserror::Error;

use super::text::TextExtractionError;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::semantic_guard::semantic_guard_route_loading::builder::RouterBuildError;

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
    Build(#[from] RouterBuildError<E>),
}

// the hook raises these out of the request as an internal error, each cause kept as its source
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
}

impl<E> From<SemanticGuardHookError<E>> for GuardrailFailure
where
    E: Error + Send + Sync + 'static,
{
    fn from(error: SemanticGuardHookError<E>) -> Self {
        Self::Http {
            status: INTERNAL_STATUS,
            detail: error.to_string(),
        }
    }
}
