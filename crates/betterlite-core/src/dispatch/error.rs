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
