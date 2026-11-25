use betterlite_config::sections::resource_sdk_settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::assistant_messages::connection::MessageParams;
use betterlite_core::api::assistant_messages::error::{MessageError, MessageFailure, SetupError};
use betterlite_core::api::assistant_messages::options::MessageData;
use betterlite_core::api::assistant_run_sdk::call::{RunFlag, RunParams, RunThreadCall};
use betterlite_core::api::assistant_run_sdk::error::{
    AssistantRunError, RunFailure, failed, map_failure,
};
use betterlite_error::envelope::WireField;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json as BodyJson;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_with::rust::double_option;

use super::error::{CallError, NativeErrorPayload, payload};

mod tests;

const UNSUPPORTED_MODEL: &str = "n/a";
const TIMEOUT_MESSAGE: &str = "Request timed out.";
const TIMEOUT_STATUS: u16 = 408;

// litellm.api_key and the other module globals _runs.py reads before every call.
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModuleSettings {
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) azure_key: Option<SecretString>,
}

impl ModuleSettings {
    // the module globals win over the environment, and one left unset keeps the environment value
    pub(crate) fn apply(&self, settings: &mut resource_sdk_settings::Settings) {
        settings.api_key.clone_from(&self.api_key);
        settings.api_base.clone_from(&self.api_base);
        settings.api_version.clone_from(&self.api_version);
        settings.organization.clone_from(&self.organization);
        settings.openai_key.clone_from(&self.openai_key);
        settings.azure_key.clone_from(&self.azure_key);
    }
}

// add_message's data and named arguments plus the connection parameters left in kwargs; get_messages carries no data.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageCall {
    pub(crate) custom_llm_provider: String,
    pub(crate) thread_id: String,
    #[serde(default)]
    pub(crate) data: Option<MessageData>,
    #[serde(default)]
    pub(crate) params: MessageParams,
    #[serde(default)]
    pub(crate) drop_params: bool,
}

// run_thread's named arguments; the Python branch that asks for the async call never reaches the wire.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunCall {
    pub(crate) custom_llm_provider: String,
    pub(crate) thread_id: String,
    pub(crate) assistant_id: String,
    #[serde(default)]
    pub(crate) additional_instructions: Option<String>,
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    #[serde(default)]
    pub(crate) metadata: Option<JsonValue>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default, with = "double_option")]
    #[expect(clippy::option_option, reason = "absent and explicit null stay apart")]
    pub(crate) stream: Option<Option<RunFlag>>,
    #[serde(default)]
    pub(crate) tools: Option<JsonValue>,
    #[serde(default)]
    pub(crate) params: RunParams,
}

impl RunCall {
    pub(crate) fn into_call(self) -> RunThreadCall {
        RunThreadCall {
            provider: self.custom_llm_provider,
            thread_id: self.thread_id,
            assistant_id: self.assistant_id,
            additional_instructions: self.additional_instructions,
            instructions: self.instructions,
            metadata: self.metadata,
            model: self.model,
            stream: self.stream,
            tools: self.tools,
            client: None,
            event_handler: None,
            arun_thread: None,
            params: self.params,
        }
    }
}

pub(crate) fn read<T>(field: &'static str, text: &str) -> Result<T, CallError>
where
    T: DeserializeOwned,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(CallError::Json { field, source }),
    }
}

// The error body openai.APIError reads type, param and code from; an absent field stays absent.
fn wire_body(fields: [(&str, &WireField); 3]) -> Option<BodyJson> {
    let mut body: IndexMap<String, BodyJson> = IndexMap::new();
    for (name, field) in fields {
        match field {
            WireField::Missing(_) => {}
            WireField::Null => {
                body.insert(name.to_owned(), BodyJson::Null);
            }
            WireField::Text(text) => {
                body.insert(name.to_owned(), BodyJson::Text(text.clone()));
            }
        }
    }
    (!body.is_empty()).then_some(BodyJson::Object(body))
}

// Each LiteLLM exception class re-adds "litellm.Class: " to the text it is built with.
fn inner_message(name: &str, message: String) -> String {
    let prefix = format!("litellm.{name}: ");
    if let Some(inner) = message.strip_prefix(&prefix) {
        return inner.to_owned();
    }
    message
}

// The class the mapping chose, with its status, provider and the body fields the SDK reads back.
fn mapped_payload(
    class: &ErrorClass,
    name: &str,
    provider: &str,
    message: String,
) -> NativeErrorPayload {
    let metadata = class.metadata();
    let code = metadata
        .openai_code
        .map_or(WireField::MISSING, WireField::Text);
    NativeErrorPayload {
        status_code: class_status(class),
        llm_provider: Some(provider.to_owned()),
        body: wire_body([
            ("type", &metadata.error_type),
            ("param", &metadata.param),
            ("code", &code),
        ]),
        ..payload(name, inner_message(name, message))
    }
}

// add_message and get_messages raise the class dispatch mapped, with the text its exception reports.
pub(crate) fn message_error(failure: &MessageFailure, provider: &str) -> NativeErrorPayload {
    let name = class_name(&failure.class);
    let message = class_message(&failure.class);
    let unsupported = matches!(
        &failure.error,
        MessageError::Setup(SetupError::UnsupportedProvider { .. })
    );
    let mut answer = mapped_payload(&failure.class, name, provider, message);
    if unsupported {
        answer.model = Some(String::from(UNSUPPORTED_MODEL));
    }
    answer
}

// The exception_type result: its class name and the message it carries.
pub(crate) fn run_failure(failure: RunFailure, provider: &str) -> NativeErrorPayload {
    mapped_payload(
        &failure.class,
        failure.class_name,
        provider,
        failure.message,
    )
}

// run_thread raises the raw provider error and arun_thread maps it; betterlite maps both the same way.
pub(crate) fn run_error(
    error: AssistantRunError,
    provider: &str,
    redactor: &Redactor,
) -> NativeErrorPayload {
    let text = error.to_string();
    match error {
        AssistantRunError::Unsupported { provider: rejected } => NativeErrorPayload {
            status_code: Some(400),
            llm_provider: Some(rejected),
            model: Some(String::from(UNSUPPORTED_MODEL)),
            ..payload("BadRequestError", text)
        },
        AssistantRunError::DuplicateStream => payload("Exception", text),
        AssistantRunError::Dispatch(dispatch) => {
            run_failure(map_failure(provider, &dispatch, redactor), provider)
        }
        AssistantRunError::Mapped(failure) => run_failure(failure, provider),
        timeout @ (AssistantRunError::Timeout(_)
        | AssistantRunError::InvalidTimeout { .. }
        | AssistantRunError::InvalidTimeoutText { .. }
        | AssistantRunError::InvalidExtraBody) => {
            let negative = matches!(timeout, AssistantRunError::InvalidTimeout { .. });
            let (message, status) = if negative {
                (String::from(TIMEOUT_MESSAGE), Some(TIMEOUT_STATUS))
            } else {
                (text, None)
            };
            run_failure(failed(provider, message, status, None, redactor), provider)
        }
    }
}
