use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::Mode;
use betterlite_openai::deepseek_chat::options::map_openai_params;
use betterlite_openai::deepseek_chat::request::{RequestInput, transform_request};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

const EVENTS: [(&str, &str); 2] = [
    ("only supports function tools", "tools_dropped"),
    ("assistant message is missing", "reasoning_placeholder"),
];

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Map { cases: Vec<MapCase> },
    Request { cases: Vec<RequestCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCase {
    model: String,
    params: Object,
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCase {
    model: String,
    messages: Vec<ChatMessage>,
    optional_params: Object,
    supports_vision: bool,
    supports_reasoning: bool,
}

fn openai_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

const fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn map_value(case: &MapCase) -> Value {
    let mut optional = case.optional_params.clone();
    map_openai_params(
        &case.model,
        &case.params,
        &mut optional,
        &openai_config(),
        &empty_catalog(),
    );
    json!(optional)
}

fn request_value(case: &RequestCase) -> Result<Value, Box<HelperError>> {
    let input = RequestInput {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional_params,
        supports_vision: case.supports_vision,
        supports_reasoning: case.supports_reasoning,
    };
    Ok(json!(
        transform_request(&input, Mode::Sync).or_config_error()?
    ))
}

fn captured<Case, Render>(logs: &Logs, key: &str, cases: &[Case], render: Render) -> HelperResult
where
    Render: Fn(&Case) -> Result<Value, Box<HelperError>>,
{
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        logs.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        let value = render(case)?;
        let warnings = std::mem::take(&mut *logs.0.lock().unwrap_or_else(PoisonError::into_inner));
        outcomes.push(json!({"value": value, "warnings": warnings}));
    }
    Ok(json!({ key: outcomes }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        with_default(subscriber, || match input {
            Input::Map { cases } => captured(&logs, "map", cases, |case| Ok(map_value(case))),
            Input::Request { cases } => captured(&logs, "request", cases, request_value),
        })
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Value>>>);

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
        let Some((_, name)) = EVENTS
            .iter()
            .find(|(fragment, _)| message.0.contains(fragment))
        else {
            return;
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": "warning", "event": name}));
    }
}
