use betterlite_core::api::assistant_messages::error::MessageFailure;
use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::AssistantSdkError;
use betterlite_core::api::assistant_records::error::AssistantCreateFailure;
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_core::api::assistant_threads::error::ThreadFailure;
use betterlite_error::envelope::WireField;
use thiserror::Error;

// The exception the SDK owner raised, with the attributes the legacy proxy handler reads off it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantFailure {
    pub class: String,
    pub status: Option<u16>,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

impl From<AssistantCreateFailure> for AssistantFailure {
    fn from(failure: AssistantCreateFailure) -> Self {
        Self {
            class: failure.class,
            status: failure.status,
            kind: failure.kind,
            param: failure.param,
            code: failure.code,
            message: failure.message,
        }
    }
}

#[derive(Debug, Error)]
pub enum AssistantDispatchError {
    // the factory's own Exception: it has no status_code, type, param or code attribute
    #[error("{message}")]
    Exception { message: String },
    // assistants_config["custom_llm_provider"] and ["litellm_params"] are plain dictionary reads
    #[error("'{key}'")]
    KeyError { key: &'static str },
    // duplicate keywords and missing required arguments are the interpreter's own errors
    #[error("{message}")]
    TypeError { message: String },
    // the typed SDK input refused a value; the message names the field
    #[error("{message}")]
    Invalid { message: String },
    // the SDK failures are boxed: each carries a whole exception, and an error stays cheap to return
    #[error("{}", failure.message)]
    Failure { failure: Box<AssistantFailure> },
    #[error(transparent)]
    ReadDelete(Box<AssistantReadDeleteError<AssistantSdkError>>),
    // get_thread reads the record's own error, not the class the facade reports
    #[error("{}", failure.error)]
    Thread { failure: Box<ThreadFailure> },
    #[error(transparent)]
    Message(Box<MessageFailure>),
    #[error(transparent)]
    Run(Box<AssistantRunError>),
}

impl AssistantDispatchError {
    // the wrapper call itself: an explicit path argument collides with the same key in the body
    #[must_use]
    pub fn multiple_values(name: &str) -> Self {
        Self::TypeError {
            message: format!(
                "litellm.router.Router.factory_function.<locals>.async_wrapper() got multiple values for keyword argument '{name}'"
            ),
        }
    }

    // the merged call into the installed function: its own named arguments collide with config parameters
    #[must_use]
    pub fn duplicate_argument(function: &str, name: &str) -> Self {
        Self::TypeError {
            message: format!(
                "litellm.assistants.main.{function}() got multiple values for keyword argument '{name}'"
            ),
        }
    }

    // CPython's own wording, singular, paired and Oxford-comma forms alike
    #[must_use]
    pub fn missing_arguments(function: &str, names: &[&str]) -> Self {
        let detail = match names {
            [one] => format!("1 required positional argument: '{one}'"),
            [first, second] => format!("2 required positional arguments: '{first}' and '{second}'"),
            many => {
                let mut listed: Vec<String> = many.iter().map(|name| format!("'{name}'")).collect();
                let last = listed.pop().unwrap_or_default();
                format!(
                    "{} required positional arguments: {}, and {last}",
                    many.len(),
                    listed.join(", ")
                )
            }
        };
        Self::TypeError {
            message: format!("{function}() missing {detail}"),
        }
    }

    #[must_use]
    pub fn invalid(field: &str) -> Self {
        Self::Invalid {
            message: format!("'{field}' has the wrong type"),
        }
    }

    // get_assistants and delete_assistant surface through the async wrapper's exception_type mapping
    #[must_use]
    pub fn read_delete(error: AssistantReadDeleteError<AssistantSdkError>) -> Self {
        let Some(fields) = error.fields() else {
            return Self::ReadDelete(Box::new(error));
        };
        let text = error.to_string();
        Self::Failure {
            failure: Box::new(AssistantFailure {
                class: fields.class.to_owned(),
                status: fields.status,
                kind: fields
                    .error_type
                    .map_or(WireField::MISSING, |kind| WireField::Text(kind.to_owned())),
                param: WireField::MISSING,
                code: WireField::MISSING,
                message: if fields.prefixed {
                    format!("litellm.{}: {text}", fields.class)
                } else {
                    text
                },
            }),
        }
    }
}
