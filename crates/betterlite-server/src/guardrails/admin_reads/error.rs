use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_config::sections::content_filter_catalog::patterns::PatternCatalogError;
use betterlite_config::sections::content_filter_catalog::roots::ContentFilterRootsError;
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_env::EnvError;
use betterlite_guardrails::guardrail_registry::error::RegistryError;
use serde::Serialize;
use thiserror::Error;

use crate::guardrails::provider_forms::error::FormError;
use crate::tracing_routes::router::unauthenticated;

#[derive(Debug, Error)]
pub(super) enum ReadError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("Guardrail with ID {guardrail_id} not found")]
    NotFound { guardrail_id: String },
    // LiteLLM's _extract_fields_recursive raises this exact text
    #[error(
        "Max depth of {limit} exceeded while processing model fields. Please check the model structure for excessive nesting."
    )]
    TooDeep { limit: usize },
    #[error("{source}")]
    Params {
        #[source]
        source: GuardrailParamsError,
    },
    #[error("{source}")]
    Registry {
        #[source]
        source: RegistryError,
    },
    #[error("{source}")]
    Identity {
        #[source]
        source: serde_json::Error,
    },
    #[error("{source}")]
    Teams {
        #[source]
        source: UserLookupError,
    },
    #[error("{source}")]
    Depth {
        #[source]
        source: EnvError,
    },
    #[error("{source}")]
    Patterns {
        #[source]
        source: PatternCatalogError,
    },
    #[error("{source}")]
    Roots {
        #[source]
        source: ContentFilterRootsError,
    },
    #[error("{source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

impl ReadError {
    const fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::TooDeep { .. } => StatusCode::BAD_REQUEST,
            Self::Params { .. }
            | Self::Registry { .. }
            | Self::Identity { .. }
            | Self::Teams { .. }
            | Self::Depth { .. }
            | Self::Patterns { .. }
            | Self::Roots { .. }
            | Self::Encode { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<GuardrailParamsError> for ReadError {
    fn from(source: GuardrailParamsError) -> Self {
        Self::Params { source }
    }
}

impl From<RegistryError> for ReadError {
    fn from(source: RegistryError) -> Self {
        Self::Registry { source }
    }
}

impl From<PatternCatalogError> for ReadError {
    fn from(source: PatternCatalogError) -> Self {
        Self::Patterns { source }
    }
}

impl From<ContentFilterRootsError> for ReadError {
    fn from(source: ContentFilterRootsError) -> Self {
        Self::Roots { source }
    }
}

impl From<serde_json::Error> for ReadError {
    fn from(source: serde_json::Error) -> Self {
        Self::Encode { source }
    }
}

impl From<FormError> for ReadError {
    fn from(error: FormError) -> Self {
        match error {
            FormError::TooDeep { limit } => Self::TooDeep { limit },
        }
    }
}

impl IntoResponse for ReadError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::NotFound { .. }
            | Self::TooDeep { .. }
            | Self::Params { .. }
            | Self::Registry { .. }
            | Self::Identity { .. }
            | Self::Teams { .. }
            | Self::Depth { .. }
            | Self::Patterns { .. }
            | Self::Roots { .. }
            | Self::Encode { .. } => {
                let detail = self.to_string();
                (self.status(), Json(DetailBody { detail })).into_response()
            }
        }
    }
}
