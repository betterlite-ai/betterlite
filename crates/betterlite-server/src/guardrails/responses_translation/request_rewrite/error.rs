use betterlite_core::api::response_chat_history::error::HistoryError;
use betterlite_core::api::responses_input_messages::error::InputMessagesError;
use betterlite_core::api::responses_tool_namespaces::error::NamespaceToolsError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;
use thiserror::Error;

use crate::guardrails::responses_translation::input_provenance::error::InputProvenanceError;
use crate::guardrails::responses_translation::tool_merge::error::ToolMergeError;

#[derive(Debug, Error)]
pub enum RequestRewriteError {
    #[error("the guardrail failed the request")]
    Guardrail {
        #[source]
        source: Box<GuardrailFailure>,
    },
    #[error("the guardrail's rewrite cannot be applied to this request")]
    Unappliable {
        #[source]
        source: UnappliableRequestRewrite,
    },
    #[error("the Responses input could not be turned into chat messages")]
    InputConversion {
        #[source]
        source: InputMessagesError,
    },
    #[error("the rewritten messages could not be turned back into Responses items")]
    History {
        #[source]
        source: HistoryError,
    },
    #[error("the returned tools could not be merged into the request")]
    ToolMerge {
        #[source]
        source: ToolMergeError,
    },
    #[error("the raw input items could not be matched to the rewritten messages")]
    Provenance {
        #[source]
        source: InputProvenanceError,
    },
    #[error("the request tools could not be read")]
    Tools {
        #[source]
        source: NamespaceToolsError,
    },
    #[error("the {target} could not be converted")]
    Convert {
        target: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the request field '{field}' must be a string")]
    FieldType { field: &'static str },
}
