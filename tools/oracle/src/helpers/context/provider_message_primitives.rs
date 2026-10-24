use std::fmt;
use std::sync::{Arc, Mutex};

use betterlite_context::error::ContextError;
use betterlite_context::provider_message_primitives::{
    BatchConfigBase, CHAT_COMPLETION_ID_PREFIX, ChoicePolicy, FormatParams, FormatSupport,
    NestedSchema, Record, ResponseFormat, Schema, StreamState, ToolCallProjection, ToolParam,
    add_response_format, developer_to_system, project_tool_call,
};
use serde_json::{Map, Value, json};
use tracing::field::Field;
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::helpers::HelperError;

mod tests;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ResponseFormat {
        optional_params: Map<String, Value>,
        value: Value,
        supported: bool,
        enforce_tool_choice: Option<bool>,
    },
    DeveloperRole {
        messages: Vec<Record<Value>>,
    },
    IteratorState {
        json_mode: JsonModeInput,
    },
    BatchInitializer,
    ChatId {
        uuid: String,
    },
    ToolCall {
        record: Record<Value>,
    },
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JsonModeInput {
    Default,
    Explicit(Option<bool>),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        let result = with_default(subscriber, || outcome(input.clone()))
            .map_err(|failure| Box::new(helper_error(&failure)))?;
        let lines = logs.0.lock().map(|lines| lines.clone()).unwrap_or_default();
        Ok(json!({"result": result, "logs": lines}))
    }
}

enum Failure {
    Context(ContextError),
    Uuid(uuid::Error),
    ToolsNotList,
}

fn outcome(input: Input) -> Result<Value, Failure> {
    match input {
        Input::ResponseFormat {
            optional_params,
            value,
            supported,
            enforce_tool_choice,
        } => response_format(optional_params, value, supported, enforce_tool_choice),
        Input::DeveloperRole { messages } => developer_to_system(messages)
            .map(|projected| json!(projected))
            .map_err(Failure::Context),
        Input::IteratorState { json_mode } => {
            let state: StreamState<usize, String> = match json_mode {
                JsonModeInput::Default => StreamState::from(0),
                JsonModeInput::Explicit(mode) => StreamState {
                    stream: 0,
                    json_mode: mode,
                    http_response: None,
                },
            };
            Ok(
                json!({"handles": 1, "reads": state.stream, "json_mode": state.json_mode, "http_response": state.http_response}),
            )
        }
        Input::BatchInitializer => Ok(json!({"attributes": size_of_val(&BatchConfigBase)})),
        Input::ChatId { uuid } => uuid::Uuid::parse_str(&uuid)
            .map(|id| json!(format!("{CHAT_COMPLETION_ID_PREFIX}{id}")))
            .map_err(Failure::Uuid),
        Input::ToolCall { record } => Ok(project_tool_call(record, &Recorder)),
    }
}

fn response_format(
    mut optional_params: Map<String, Value>,
    value: Value,
    supported: bool,
    enforce_tool_choice: Option<bool>,
) -> Result<Value, Failure> {
    let support = if supported {
        FormatSupport::Native
    } else {
        FormatSupport::ToolCall
    };
    let policy = match enforce_tool_choice {
        Some(false) => ChoicePolicy::KeepExisting,
        Some(true) | None => ChoicePolicy::Enforce,
    };
    let tools = match optional_params.remove("tools") {
        None => None,
        Some(Value::Array(tools)) => Some(tools.into_iter().map(ToolParam::Supplied).collect()),
        Some(_) => return Err(Failure::ToolsNotList),
    };
    let mut params = FormatParams {
        tools,
        tool_choice: None,
        json_mode: None,
        response_format: None,
    };
    add_response_format(&mut params, format_record(value), support, policy)
        .map_err(Failure::Context)?;
    if let Ok(Value::Object(changed)) = serde_json::to_value(&params) {
        optional_params.extend(changed);
    }
    Ok(Value::Object(optional_params))
}

fn format_record(value: Value) -> ResponseFormat<Value> {
    let json_schema = value.get("json_schema").map(|nested| {
        let object = nested.as_object();
        object.map_or(NestedSchema::NotObject, |fields| {
            NestedSchema::Object(fields.get("schema").map(schema))
        })
    });
    ResponseFormat {
        response_schema: value.get("response_schema").map(schema),
        json_schema,
        value,
    }
}

// Python truthiness decides which schemas count as absent
fn schema(field: &Value) -> Schema<Value> {
    let truthy = match field {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float.abs() > 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    };
    if truthy {
        Schema::Truthy(field.clone())
    } else {
        Schema::Falsy
    }
}

struct Recorder;

impl ToolCallProjection<Value> for Recorder {
    type Output = Value;

    fn custom(&self, fields: Record<Value>) -> Value {
        json!({"kind": "custom", "fields": fields})
    }

    fn function(&self, fields: Record<Value>) -> Value {
        json!({"kind": "function", "fields": fields})
    }
}

fn helper_error(failure: &Failure) -> HelperError {
    let (class, message) = match failure {
        Failure::Context(error @ ContextError::SchemaNotObject) => ("TypeError", error.to_string()),
        Failure::Context(
            error @ (ContextError::MissingSchema
            | ContextError::MissingRole { .. }
            | ContextError::MissingContent { .. }),
        ) => ("KeyError", error.to_string()),
        Failure::Uuid(error) => ("ValueError", error.to_string()),
        Failure::ToolsNotList => ("AttributeError", "tools is not a list".to_owned()),
    };
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Value>>>);

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = String::new();
        event.record(&mut |field: &Field, value: &dyn fmt::Debug| {
            if field.name() == "message" {
                message = format!("{value:?}");
            }
        });
        let level = event.metadata().level().to_string();
        if let Ok(mut lines) = self.0.lock() {
            lines.push(json!({"level": level, "message": message}));
        }
    }
}
