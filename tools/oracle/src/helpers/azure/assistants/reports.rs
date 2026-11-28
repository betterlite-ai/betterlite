use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

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

// What the Python helper reports for the errors the Azure SDK and its own path building raise.
pub(super) fn failure(error: &AssistantsRunError) -> Result<Value, Box<HelperError>> {
    match error {
        AssistantsRunError::Runs(ThreadRunsError::Status(status, _)) => {
            Ok(json!({"kind": "status", "status": status}))
        }
        AssistantsRunError::Client(DefinitionsError::MissingCredentials) => {
            Ok(json!({"kind": "configuration"}))
        }
        AssistantsRunError::Runs(
            ThreadRunsError::EmptyThreadId
            | ThreadRunsError::EmptyRunId
            | ThreadRunsError::DotSegment(_),
        ) => Ok(json!({"kind": "value"})),
        other @ (AssistantsRunError::Client(_)
        | AssistantsRunError::Runs(_)
        | AssistantsRunError::Encode(_)) => Err(invalid_input(other.to_string())),
    }
}
