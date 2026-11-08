use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Messages {
    Text(String),
    Values(Vec<Json>),
}

impl Messages {
    // LiteLLM wraps a bare string, and every element when the first one is a string, as a user message
    pub fn normalized(&self) -> Vec<Json> {
        match self {
            Self::Text(text) => vec![user_message(Json::Text(text.clone()))],
            Self::Values(values) => match values.first() {
                Some(Json::Text(_)) => values.iter().cloned().map(user_message).collect(),
                _ => values.clone(),
            },
        }
    }
}

impl From<&Messages> for Json {
    fn from(messages: &Messages) -> Self {
        match messages {
            Messages::Text(text) => Self::Text(text.clone()),
            Messages::Values(values) => Self::List(values.clone()),
        }
    }
}

fn user_message(content: Json) -> Json {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert("content".to_owned(), content);
    Json::Object(message)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct DynamicCallbacks {
    pub input: Option<Vec<String>>,
    pub success: Option<Vec<String>>,
    pub async_success: Option<Vec<String>>,
    pub failure: Option<Vec<String>>,
    pub async_failure: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct RequestLoggingFlags {
    pub log_raw_request_response: bool,
    pub raw_request_only: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CallbackPolicy {
    pub global_disable_no_log_param: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CostState {
    pub custom_pricing: bool,
    pub zero_cost_warned: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeferredLogging {
    pub defer_async_logging: bool,
    pub async_success_scheduled: bool,
}
