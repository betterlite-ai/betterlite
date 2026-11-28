use std::mem;
use std::sync::PoisonError;

use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_thread_messages::error::ThreadMessagesError;
use betterlite_azure::azure_thread_messages::operations::ThreadReply;
use betterlite_openai::threads::error::ThreadsError;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::Recorder;

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// What the Python helper reports for the errors the Azure SDK raises.
pub(super) fn failure(error: &ThreadMessagesError) -> Result<Value, Box<HelperError>> {
    match error {
        ThreadMessagesError::Threads(ThreadsError::Status { status, .. }) => {
            Ok(json!({"kind": "status", "status": status}))
        }
        ThreadMessagesError::Client(DefinitionsError::MissingCredentials) => {
            Ok(json!({"kind": "configuration"}))
        }
        ThreadMessagesError::Client(
            DefinitionsError::MissingEndpoint
            | DefinitionsError::ProviderReply
            | DefinitionsError::Provider { .. }
            | DefinitionsError::Timeout { .. },
        )
        | ThreadMessagesError::Threads(
            ThreadsError::EmptyThreadId
            | ThreadsError::DotSegment { .. }
            | ThreadsError::Reply { .. },
        ) => Ok(json!({"kind": "value"})),
        other @ (ThreadMessagesError::Client(_) | ThreadMessagesError::Threads(_)) => {
            Err(invalid_input(other.to_string()))
        }
    }
}

// The facade's own reply and kind, plus everything the wire saw before answering.
pub(super) fn operation(
    recorder: &Recorder,
    outcome: Result<ThreadReply, ThreadMessagesError>,
) -> HelperResult {
    let requests = mem::take(&mut *recorder.seen.lock().unwrap_or_else(PoisonError::into_inner));
    match outcome {
        Ok(reply) => Ok(json!({
            "requests": requests,
            "kind": reply.kind.label(),
            "result": reply.thread,
        })),
        Err(error) => Ok(json!({"requests": requests, "error": failure(&error)?})),
    }
}
