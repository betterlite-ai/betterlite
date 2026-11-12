use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{ChatContentError, Mode};
use betterlite_openai::deepseek_chat::history::fill_reasoning_content;
use betterlite_openai::deepseek_chat::messages::transform_messages;
use betterlite_openai::deepseek_chat::options::{map_openai_params, supported_openai_params};
use betterlite_openai::deepseek_chat::request::{RequestInput, transform_request};
use betterlite_openai::deepseek_chat::tools::drop_unsupported_tools;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

const EVENTS: [(&str, &str); 2] = [
    ("only supports function tools", "tools_dropped"),
    ("assistant message is missing", "reasoning_placeholder"),
];

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Supported { cases: Vec<SupportedCase> },
    Map { cases: Vec<MapCase> },
    History { cases: Vec<HistoryCase> },
    Messages { cases: Vec<MessagesCase> },
    Tools { cases: Vec<ToolsCase> },
    Request { cases: Vec<RequestCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportedCase {
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCase {
    model: String,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCase {
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessagesCase {
    model: String,
    messages: Vec<ChatMessage>,
    supports_vision: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsCase {
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCase {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    optional_params: Object,
    supports_vision: bool,
    supports_reasoning: bool,
    #[serde(default)]
    asynchronous: bool,
}

fn openai_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn content_error(error: ChatContentError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("BadRequestError"),
        status: Some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn map_value(case: &MapCase) -> Result<Value, Box<HelperError>> {
    let mut optional = case.optional_params.clone();
    map_openai_params(
        &case.model,
        &case.params,
        &mut optional,
        &openai_config(),
        &empty_catalog(),
    );
    Ok(json!(optional))
}

fn messages_value(case: &MessagesCase) -> Result<Value, Box<HelperError>> {
    let transformed = transform_messages(&case.model, &case.messages, case.supports_vision)
        .map_err(content_error)?;
    Ok(json!(transformed))
}

fn request_value(case: &RequestCase) -> Result<Value, Box<HelperError>> {
    let input = RequestInput {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional_params,
        supports_vision: case.supports_vision,
        supports_reasoning: case.supports_reasoning,
    };
    let mode = if case.asynchronous {
        Mode::AsyncSubclass
    } else {
        Mode::Sync
    };
    let body = transform_request(&input, mode).map_err(content_error)?;
    Ok(json!(body))
}

fn captured<Case, Render>(
    logs: &Logs,
    cases: &[Case],
    render: Render,
) -> Result<Vec<Value>, Box<HelperError>>
where
    Render: Fn(&Case) -> Result<Value, Box<HelperError>>,
{
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let _ = logs.take();
        let value = render(case)?;
        outcomes.push(json!({"value": value, "warnings": logs.take()}));
    }
    Ok(outcomes)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        with_default(subscriber, || match input {
            Input::Supported { cases } => captured(&logs, cases, |case| {
                Ok(json!(supported_openai_params(
                    &case.model,
                    &openai_config(),
                    &empty_catalog()
                )))
            })
            .map(|values| json!({"supported": values})),
            Input::Map { cases } => {
                captured(&logs, cases, map_value).map(|values| json!({"mapped": values}))
            }
            Input::History { cases } => captured(&logs, cases, |case| {
                Ok(json!(fill_reasoning_content(&case.messages)))
            })
            .map(|values| json!({"history": values})),
            Input::Messages { cases } => {
                captured(&logs, cases, messages_value).map(|values| json!({"messages": values}))
            }
            Input::Tools { cases } => captured(&logs, cases, |case| {
                Ok(json!(drop_unsupported_tools(&case.optional_params).params))
            })
            .map(|values| json!({"tools": values})),
            Input::Request { cases } => {
                captured(&logs, cases, request_value).map(|values| json!({"bodies": values}))
            }
        })
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Value>>>);

impl Logs {
    fn take(&self) -> Vec<Value> {
        let mut taken = std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
        taken.retain(|entry| entry["event"] != "unrecognized");
        taken
    }
}

#[derive(Default)]
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Fields::default();
        event.record(&mut message);
        let name = EVENTS
            .iter()
            .find(|(fragment, _)| message.0.contains(fragment))
            .map_or("unrecognized", |(_, name)| name);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": "warning", "event": name}));
    }
}
