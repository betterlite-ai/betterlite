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

mod tests;

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

fn text_field(value: Option<String>) -> WireField {
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

// A sync facade raises the SDK error itself, while its async twin raises the class exception_type mapped.
pub(crate) enum ThreadOutcome {
    Sync(ThreadError),
    Async(ThreadFailure),
}

// What both thread maps know before they build their payload.
struct ThreadFields<'class> {
    class: &'class str,
    status: Option<u16>,
    fields: [WireField; 3],
    message: String,
    unsupported: bool,
}

fn thread_body(fields: &[WireField; 3]) -> Option<Json> {
    wire_body(&[
        ("type", &fields[0]),
        ("param", &fields[1]),
        ("code", &fields[2]),
    ])
}

fn thread_error(fields: ThreadFields<'_>, provider: &str) -> NativeErrorPayload {
    let mut answer = payload(fields.class, fields.message);
    answer.status_code = fields.status;
    answer.llm_provider = Some(provider.to_owned());
    answer.model = fields
        .unsupported
        .then_some(String::from(UNSUPPORTED_MODEL));
    answer.body = thread_body(&fields.fields);
    answer
}

// create_thread and get_thread raise the SDK exception itself, their async twins the class exception_type mapped.
pub(crate) fn thread(outcome: &ThreadOutcome, provider: &str) -> NativeErrorPayload {
    let fields = match outcome {
        ThreadOutcome::Sync(error) => {
            let failure = error.sdk_failure();
            let message = error.to_string();
            let unsupported =
                failure.class == "BadRequestError" && message.starts_with(UNSUPPORTED_PREFIX);
            ThreadFields {
                class: failure.class,
                status: Some(failure.status),
                fields: [
                    text_field(failure.kind),
                    text_field(failure.param),
                    text_field(failure.code),
                ],
                message,
                unsupported,
            }
        }
        ThreadOutcome::Async(failure) => {
            let metadata = failure.class.metadata();
            let message = failure.error.to_string();
            let unsupported = message.starts_with(UNSUPPORTED_PREFIX);
            ThreadFields {
                class: class_name(&failure.class),
                status: class_status(&failure.class),
                fields: [
                    metadata.error_type,
                    metadata.param,
                    text_field(metadata.openai_code),
                ],
                message,
                unsupported,
            }
        }
    };
    thread_error(fields, provider)
}
