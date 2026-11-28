use std::mem;
use std::sync::PoisonError;

use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_azure::azure_message_attachments::legacy::{
    ParamsError, SUPPORTED_MESSAGE_PARAMS, map_create_message_params,
};
use betterlite_azure::azure_message_attachments::operations::{MessageReply, MessagesReply};
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::threads::error::ThreadsError;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::MapStep;
use super::fixture::Wire;

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

// What the Python helper reports for the errors the Azure SDK and LiteLLM raise.
pub(super) fn failure(error: &MessagesError) -> Result<Value, Box<HelperError>> {
    match error {
        MessagesError::Definitions(DefinitionsError::Assistants(AssistantsError::Status {
            status,
            ..
        }))
        | MessagesError::Threads(ThreadsError::Status { status, .. }) => {
            Ok(json!({"kind": "status", "status": status}))
        }
        MessagesError::Definitions(DefinitionsError::MissingCredentials) => {
            Ok(json!({"kind": "configuration"}))
        }
        MessagesError::Definitions(
            DefinitionsError::Assistants(
                AssistantsError::EmptyAssistantId | AssistantsError::DotSegment { .. },
            )
            | DefinitionsError::MissingEndpoint
            | DefinitionsError::ProviderReply
            | DefinitionsError::Timeout { .. },
        )
        | MessagesError::Threads(
            ThreadsError::EmptyThreadId
            | ThreadsError::DotSegment { .. }
            | ThreadsError::Reply { .. },
        ) => Ok(json!({"kind": "value"})),
        MessagesError::Definitions(_) | MessagesError::Threads(_) => {
            Err(invalid_input(error.to_string()))
        }
    }
}

// The facade's own reply and kind, plus everything the wire saw before answering.
pub(super) fn operation(
    wire: &Wire,
    outcome: Result<MessagesReply, MessagesError>,
) -> HelperResult {
    let requests = mem::take(&mut *wire.seen.lock().unwrap_or_else(PoisonError::into_inner));
    match outcome {
        Ok(reply) => {
            let result = match &reply.reply {
                MessageReply::Message(message) => serde_json::to_value(message.as_ref()),
                MessageReply::Page(page) => serde_json::to_value(page.as_ref()),
            };
            Ok(json!({
                "requests": requests,
                "kind": reply.kind.label(),
                "result": result.unwrap_or(Value::Null),
            }))
        }
        Err(error) => Ok(json!({"requests": requests, "error": failure(&error)?})),
    }
}

// One mapping call reports the supported list it read and the dict LiteLLM returned.
pub(super) fn mapped(step: &MapStep) -> HelperResult {
    let mut optional = step.optional.clone();
    map_create_message_params(&step.non_default, &mut optional, step.drop_params)
        .map_err(|error| mapped_failure(&error))?;
    Ok(json!({"supported": SUPPORTED_MESSAGE_PARAMS, "optional": optional}))
}

// LiteLLM raises here, so the failure leaves the helper as the error envelope its Python side gets.
fn mapped_failure(error: &ParamsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("UnsupportedParamsError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
