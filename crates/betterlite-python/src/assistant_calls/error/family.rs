use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::AssistantSdkError;
use betterlite_core::api::assistant_records::error::{
    AssistantCreateFailure, Entry, Raised, raised,
};
use betterlite_core::api::assistant_threads::error::{ThreadError, ThreadFailure};
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;

use super::{NativeErrorPayload, payload};

// litellm's own errors for an unsupported provider carry this model instead of the caller's
const UNSUPPORTED_MODEL: &str = "n/a";
const UNSUPPORTED_PREFIX: &str = "LiteLLM doesn't support ";
const INTERNAL_TYPE: &str = "internal_server_error";

// The error body openai.APIError reads type, param and code from; absent fields stay absent.
fn wire_body(fields: &[(&str, &WireField)]) -> Option<Json> {
    let mut body: IndexMap<String, Json> = IndexMap::new();
    for (name, field) in fields {
        match field {
            WireField::Missing(_) => {}
            WireField::Null => {
                body.insert((*name).to_owned(), Json::Null);
            }
            WireField::Text(text) => {
                body.insert((*name).to_owned(), Json::Text(text.clone()));
            }
        }
    }
    if body.is_empty() {
        None
    } else {
        Some(Json::Object(body))
    }
}

const fn text_field(value: Option<String>) -> WireField {
    match value {
        Some(text) => WireField::Text(text),
        None => WireField::MISSING,
    }
}

// The create façades hand over the class and status, and their own failures name the model instead of n/a.
pub(crate) fn create(
    failure: &AssistantCreateFailure,
    request_model: &str,
    provider: &str,
) -> NativeErrorPayload {
    let unsupported =
        failure.class == "BadRequestError" && failure.message.starts_with(UNSUPPORTED_PREFIX);
    let model = if unsupported {
        String::from(UNSUPPORTED_MODEL)
    } else {
        request_model.to_owned()
    };
    NativeErrorPayload {
        status_code: failure.status,
        llm_provider: Some(provider.to_owned()),
        model: Some(model),
        body: wire_body(&[
            ("type", &failure.kind),
            ("param", &failure.param),
            ("code", &failure.code),
        ]),
        ..payload(&failure.class, failure.message.clone())
    }
}

// get_aget_assistants and delete_adelete_assistant run the same facade, whose dispatch failures are SDK ones.
pub(crate) fn read_delete(
    error: AssistantReadDeleteError<AssistantSdkError>,
    provider: &str,
    redactor: &Redactor,
) -> NativeErrorPayload {
    let message = error.to_string();
    match error {
        AssistantReadDeleteError::Unsupported {
            provider: rejected, ..
        } => NativeErrorPayload {
            status_code: Some(400),
            llm_provider: Some(rejected),
            model: Some(String::from(UNSUPPORTED_MODEL)),
            ..payload("BadRequestError", message)
        },
        AssistantReadDeleteError::InvalidFlag { .. }
        | AssistantReadDeleteError::MissingAssistantId
        | AssistantReadDeleteError::Limits(_)
        | AssistantReadDeleteError::Timeout(_) => payload("Exception", message),
        AssistantReadDeleteError::MissingReply { .. }
        | AssistantReadDeleteError::MismatchedReply => NativeErrorPayload {
            status_code: Some(500),
            llm_provider: Some(provider.to_owned()),
            model: Some(String::from(UNSUPPORTED_MODEL)),
            body: wire_body(&[("type", &WireField::Text(String::from(INTERNAL_TYPE)))]),
            ..payload("InternalServerError", message)
        },
        AssistantReadDeleteError::Dispatch(source) => {
            let origin = match source {
                AssistantSdkError::Assistants(error) => Raised::Assistants(error),
                AssistantSdkError::Definitions(error) => Raised::from(error),
            };
            // the async twin maps a provider failure with model="", and betterlite has one path for both
            let failure = raised(&origin, "", provider, Entry::Sync, redactor);
            create(&failure, "", provider)
        }
    }
}

// create_thread and get_thread raise the SDK exception itself, mapped to its class, status and body.
pub(crate) fn thread_sync(error: &ThreadError, provider: &str) -> NativeErrorPayload {
    let failure = error.sdk_failure();
    let message = error.to_string();
    let unsupported = failure.class == "BadRequestError" && message.starts_with(UNSUPPORTED_PREFIX);
    NativeErrorPayload {
        status_code: Some(failure.status),
        llm_provider: Some(provider.to_owned()),
        model: unsupported.then_some(String::from(UNSUPPORTED_MODEL)),
        body: wire_body(&[
            ("type", &text_field(failure.kind)),
            ("param", &text_field(failure.param)),
            ("code", &text_field(failure.code)),
        ]),
        ..payload(failure.class, message)
    }
}

// acreate_thread and aget_thread run exception_type over the same failure, so its class is already mapped.
pub(crate) fn thread_async(failure: &ThreadFailure, provider: &str) -> NativeErrorPayload {
    let metadata = failure.class.metadata();
    let unsupported = failure.error.to_string().starts_with(UNSUPPORTED_PREFIX);
    NativeErrorPayload {
        status_code: class_status(&failure.class),
        llm_provider: Some(provider.to_owned()),
        model: unsupported.then_some(String::from(UNSUPPORTED_MODEL)),
        body: wire_body(&[
            ("type", &metadata.error_type),
            ("param", &metadata.param),
            ("code", &text_field(metadata.openai_code)),
        ]),
        ..payload(class_name(&failure.class), failure.error.to_string())
    }
}
