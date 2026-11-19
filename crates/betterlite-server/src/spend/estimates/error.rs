use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;
use thiserror::Error;

const INVALID_REQUEST_TYPE: &str = "invalid_request_error";
const INTERNAL_SERVER_ERROR_TYPE: &str = "internal_server_error";
const MODEL_PARAM: &str = "model";

#[derive(Debug, Error)]
pub enum EstimateError {
    #[error("the request body does not match the declared types")]
    Request(Vec<Issue>),
    // the two HTTPExceptions calculate_spend raises itself; getattr(e, "type", "None") keeps the string
    #[error("Bad Request - messages must be provided if 'model' is provided")]
    MessagesRequired,
    #[error("Bad Request - Either 'model' or 'completion_response' must be provided")]
    InputRequired,
    #[error("{message}")]
    ModelNotMapped { message: String },
    #[error("{message}")]
    Failure { message: String },
    // HTTPException(500, detail={"error": "No llm_router found"}) stringifies its detail for ProxyException
    #[error("{{'error': 'No llm_router found'}}")]
    NoRouter,
    #[error(
        "No provider budget config found. Please set a provider budget config in the router settings. https://docs.litellm.ai/docs/proxy/provider_budget_routing"
    )]
    NoProviderBudgetConfig,
    #[error("No router budget logger found")]
    NoBudgetLimiter,
    #[error("{message}")]
    BudgetRead { message: String },
}

// LiteLLM's validation handler sends only type, loc and msg.
#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<String>,
    pub msg: String,
}

#[derive(Debug, Serialize)]
struct DetailBody {
    detail: Vec<Issue>,
}

impl IntoResponse for EstimateError {
    fn into_response(self) -> Response {
        match self {
            Self::Request(issues) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(DetailBody { detail: issues }),
            )
                .into_response(),
            Self::MessagesRequired | Self::InputRequired => proxy(
                self.to_string(),
                WireField::MISSING,
                WireField::MISSING,
                StatusCode::BAD_REQUEST,
            ),
            Self::ModelNotMapped { .. } => proxy(
                self.to_string(),
                WireField::Text(INVALID_REQUEST_TYPE.to_owned()),
                WireField::Text(MODEL_PARAM.to_owned()),
                StatusCode::BAD_REQUEST,
            ),
            Self::Failure { .. } => proxy(
                self.to_string(),
                WireField::MISSING,
                WireField::MISSING,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            Self::NoRouter
            | Self::NoProviderBudgetConfig
            | Self::NoBudgetLimiter
            | Self::BudgetRead { .. } => proxy(
                self.to_string(),
                WireField::Text(INTERNAL_SERVER_ERROR_TYPE.to_owned()),
                WireField::Null,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}

// ProxyException keeps the code as text and openai_exception_handler turns it back into the HTTP status
fn proxy(message: String, error_type: WireField, param: WireField, status: StatusCode) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message,
            error_type,
            param,
            code: WireField::Text(status.as_u16().to_string()),
        },
    };
    (status, Json(body)).into_response()
}
