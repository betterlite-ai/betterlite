use betterlite_models::billing_text::error::BillingTextError;
use reqwest::StatusCode;
use serde::Serialize;
use thiserror::Error;
use tokio::task::JoinError;

pub const REJECTED_MESSAGE: &str = "Rejected message. This is a prompt injection attack.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RejectionDetail {
    pub error: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub status: StatusCode,
    pub detail: RejectionDetail,
}

#[derive(Debug, Error)]
pub enum PromptInjectionError {
    #[error("{}", REJECTED_MESSAGE)]
    Rejected,
    #[error(
        "PromptInjectionDetection: Model List not set. Required for Prompt Injection detection."
    )]
    ModelListNotSet,
    #[error(
        "PromptInjectionDetection: Invalid LLM API Name. LLM API Name must be a 'model_name' in 'model_list'."
    )]
    InvalidLlmApiName,
    #[error("the prompt for the prompt injection hook could not be built")]
    Prompt {
        #[source]
        source: BillingTextError,
    },
    #[error("the prompt injection heuristics pool is closed")]
    HeuristicsClosed,
    #[error("a prompt injection heuristics check failed")]
    HeuristicsWorker {
        #[source]
        source: JoinError,
    },
    #[error("the prompt injection judge call failed")]
    Judge,
}

impl PromptInjectionError {
    #[must_use]
    pub const fn rejection(&self) -> Option<Rejection> {
        match self {
            Self::Rejected => Some(Rejection {
                status: StatusCode::BAD_REQUEST,
                detail: RejectionDetail {
                    error: REJECTED_MESSAGE,
                },
            }),
            Self::ModelListNotSet
            | Self::InvalidLlmApiName
            | Self::Prompt { .. }
            | Self::HeuristicsClosed
            | Self::HeuristicsWorker { .. }
            | Self::Judge => None,
        }
    }
}
