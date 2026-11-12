use betterlite_models::call_helpers::expected_error::{ClientErrorFacts, is_expected_client_error};
use betterlite_models::decisions::Json;
use serde_json::Number;
use thiserror::Error;

use crate::operation::chat::ChatFailure;
use crate::operation::embedding::EmbeddingFailure;
use crate::routes::CallType;
use crate::rules::RuleError;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("betterlite does not dispatch the {entry} call yet")]
    NotBuilt { entry: &'static str },
    #[error("no dispatch entry serves the {call_type:?} call type")]
    UnknownOperation { call_type: CallType },
    #[error("the {call_type:?} call type does not reach the {entry} entry")]
    MismatchedOperation {
        call_type: CallType,
        entry: &'static str,
    },
    #[error("no model_list entry has the id {model_id}")]
    MissingDeployment { model_id: String },
    #[error("call {call_id} passed its deadline before it started")]
    DeadlineExpired { call_id: String },
    #[error("a pre-call rule rejected the request: {source}")]
    Rules {
        #[source]
        source: RuleError,
    },
    #[error(transparent)]
    Chat {
        #[from]
        source: ChatFailure,
    },
    #[error(transparent)]
    Embedding {
        #[from]
        source: EmbeddingFailure,
    },
    // the route-shaped answer an entry names itself, for call types with no provider error to map
    #[error("{message}")]
    Rejected {
        status: u16,
        error_type: String,
        param: Option<String>,
        code: String,
        message: String,
    },
}

impl DispatchError {
    fn status(&self) -> u16 {
        match self {
            Self::Chat { source } => source.status(),
            Self::Embedding { source } => source.status(),
            Self::MissingDeployment { .. } => 400,
            Self::DeadlineExpired { .. } => 408,
            Self::Rejected { status, .. } => *status,
            Self::NotBuilt { .. }
            | Self::UnknownOperation { .. }
            | Self::MismatchedOperation { .. }
            | Self::Rules { .. } => 500,
        }
    }

    const fn provider_originated(&self) -> bool {
        match self {
            Self::Chat { source } => source.provider_originated(),
            Self::Embedding { source } => source.provider_originated(),
            Self::NotBuilt { .. }
            | Self::UnknownOperation { .. }
            | Self::MismatchedOperation { .. }
            | Self::MissingDeployment { .. }
            | Self::DeadlineExpired { .. }
            | Self::Rules { .. }
            | Self::Rejected { .. } => false,
        }
    }

    // LiteLLM logs a client error without the traceback a provider failure gets
    #[must_use]
    pub fn expected_client_failure(&self) -> bool {
        let facts = ClientErrorFacts {
            code: None,
            status_code: Some(Json::Number(Number::from(u64::from(self.status())))),
            llm_provider: self
                .provider_originated()
                .then_some(String::from("provider")),
            category: None,
            http_exception: false,
            base_llm_exception: false,
        };
        is_expected_client_error(Some(&facts))
    }
}
