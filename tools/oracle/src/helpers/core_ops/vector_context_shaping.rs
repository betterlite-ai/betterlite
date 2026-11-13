use betterlite_core::api::vector_context_shaping::{
    FailureMode, configured_failure_mode, context_messages, extract_query_from_messages,
    messages_with_context, requested_vector_store_ids, scan_request,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::chat::ChatMessage;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    QueryUsesLastMessage {
        messages: Vec<ChatMessage>,
    },
    ContextTextAndPosition {
        search_results: Vec<VectorStoreSearchResponse>,
        messages: Vec<ChatMessage>,
    },
    ScanRequestMerge {
        model: String,
        non_default_params: IndexMap<String, Json>,
    },
    RequestedIdsForDiagnostics {
        non_default_params: IndexMap<String, Json>,
    },
    FailurePolicy {
        configured: Json,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::QueryUsesLastMessage { messages } => {
                Ok(json!(extract_query_from_messages(messages)))
            }
            Input::ContextTextAndPosition {
                search_results,
                messages,
            } => {
                let context = context_messages(search_results);
                let with_context = messages_with_context(messages, &context);
                Ok(json!({
                    "context_messages": context,
                    "messages_with_context": with_context,
                }))
            }
            Input::ScanRequestMerge {
                model,
                non_default_params,
            } => Ok(json!(scan_request(model, non_default_params))),
            Input::RequestedIdsForDiagnostics { non_default_params } => {
                Ok(json!(requested_vector_store_ids(non_default_params)))
            }
            Input::FailurePolicy { configured } => {
                let resolution = configured_failure_mode(configured);
                let mode = match resolution.mode {
                    FailureMode::Annotate => "annotate",
                    FailureMode::Error => "error",
                };
                Ok(json!({"mode": mode, "warning": resolution.warning}))
            }
        }
    }
}
