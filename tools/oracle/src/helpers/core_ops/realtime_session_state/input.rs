use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use super::Failure;
use super::failure;
use super::provider::ProviderInput;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) logged_event_types: Option<Value>,
    #[serde(default)]
    pub(crate) client_wants_beta: bool,
    #[serde(default)]
    pub(crate) backend_uses_beta_protocol: Option<bool>,
    #[serde(default)]
    pub(crate) force_transcription_model: Option<String>,
    #[serde(default)]
    pub(crate) request_data: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) user_api_key_dict: Option<Json>,
    #[serde(default)]
    pub(crate) provider: Option<ProviderInput>,
    #[serde(default)]
    pub(crate) store: Vec<Frame>,
    #[serde(default)]
    pub(crate) inputs: Vec<InputMessage>,
    #[serde(default)]
    pub(crate) backend_events: Vec<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) usage_events: Vec<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) flush: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Frame {
    Text { text: String },
    Bytes { text: String },
    Record { values: IndexMap<String, Json> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum InputMessage {
    Text { text: String },
    Record { values: IndexMap<String, Json> },
}

pub(crate) fn selection(value: Option<&Value>) -> Result<LoggedEventTypes, Failure> {
    match value {
        None => Ok(LoggedEventTypes::Default),
        Some(Value::String(text)) if text == "*" => Ok(LoggedEventTypes::All),
        Some(Value::Array(items)) => selection_names(items),
        Some(_) => Err(failure(
            "InvalidInput",
            "logged_event_types is null, \"*\" or a list of event names".to_owned(),
        )),
    }
}

fn selection_names(items: &[Value]) -> Result<LoggedEventTypes, Failure> {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(name) = item else {
            return Err(failure(
                "InvalidInput",
                "every logged event type is a string".to_owned(),
            ));
        };
        names.push(name.clone());
    }
    Ok(LoggedEventTypes::Only(names))
}
