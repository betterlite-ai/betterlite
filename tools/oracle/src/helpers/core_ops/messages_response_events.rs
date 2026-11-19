use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_options::refusal::OutputItem;
use betterlite_core::api::messages_response_events::failure::{
    anthropic_error_chunk, failure_status_and_message,
};
use betterlite_core::api::messages_response_events::input::{
    ErrorAttribute, EventAttributes, ParsedEvent, ResponseAttribute, ResponseAttributes,
};
use betterlite_core::api::messages_response_events::{MessageStreamSnapshot, MessageStreamState};
use betterlite_core::api::responses_stream_errors::failure::FailureCause;
use betterlite_core::api::responses_stream_primitives::error_fields::StreamErrorAttributes;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Redaction {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EventShape {
    Mapping,
    Attributes,
    Attributed,
}

#[derive(Debug, Deserialize)]
struct EventSpec {
    shape: EventShape,
    fields: Option<IndexMap<String, Json>>,
    attributes: Option<AttributesSpec>,
}

#[derive(Debug, Deserialize)]
struct AttributesSpec {
    #[serde(rename = "type")]
    event_type: Option<Json>,
    item: Option<Json>,
    item_id: Option<Json>,
    delta: Option<Json>,
    summary_index: Option<Json>,
    response: Option<ResponseSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
enum ResponseSpec {
    Object {
        #[serde(default)]
        output: Vec<Json>,
        status: Option<String>,
        usage: Option<ResponseApiUsage>,
        error: Option<ErrorSpec>,
    },
    Value {
        value: Json,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
enum ErrorSpec {
    Object {
        message: Option<Json>,
        #[serde(rename = "type")]
        error_type: Option<Json>,
        code: Option<Json>,
    },
    Value {
        value: Json,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    ProcessEvent { event: Box<EventSpec> },
    ProjectFailure { failure: Box<FailureCause> },
    ErrorChunk { status_code: u16, message: String },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    message_id: String,
    model: String,
    redaction: Redaction,
    steps: Vec<Step>,
}

#[derive(serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum StepResult {
    ProcessEvent { failed: bool },
    ProjectFailure { status: u16, message: String },
    ErrorChunk { chunk: Json },
}

#[derive(serde::Serialize)]
struct Output {
    results: Vec<StepResult>,
    state: MessageStreamSnapshot,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let redactor = build_redactor(input.redaction)?;
        let mut state = MessageStreamState::new(
            input.message_id.clone(),
            input.model.clone(),
            redactor.clone(),
            ApiLimits::default(),
        );
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(run_step(&mut state, step, redactor.as_deref())?);
        }
        let output = Output {
            results,
            state: state.snapshot(),
        };
        serde_json::to_value(output).or_value_error()
    }
}

fn run_step(
    state: &mut MessageStreamState,
    step: &Step,
    redactor: Option<&Redactor>,
) -> Result<StepResult, Box<HelperError>> {
    match step {
        Step::ProcessEvent { event } => {
            process_event(state, event)?;
            Ok(StepResult::ProcessEvent {
                failed: state.snapshot().stream_failed.0,
            })
        }
        Step::ProjectFailure { failure } => {
            let (status, message) = failure_status_and_message(failure);
            Ok(StepResult::ProjectFailure { status, message })
        }
        Step::ErrorChunk {
            status_code,
            message,
        } => {
            let chunk =
                anthropic_error_chunk(*status_code, message, redactor, ApiLimits::default())
                    .or_value_error()?;
            Ok(StepResult::ErrorChunk { chunk })
        }
    }
}

fn process_event(state: &mut MessageStreamState, spec: &EventSpec) -> Result<(), Box<HelperError>> {
    let empty = IndexMap::new();
    let fields = spec.fields.as_ref().unwrap_or(&empty);
    let output = match spec
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.response.as_ref())
    {
        Some(ResponseSpec::Object { output: items, .. }) => {
            items.iter().map(OutputItem::Raw).collect()
        }
        Some(ResponseSpec::Value { .. }) | None => Vec::new(),
    };
    let attributes = build_attributes(spec, &output);
    let event = match spec.shape {
        EventShape::Mapping => ParsedEvent {
            attributes: EventAttributes::default(),
            mapping: Some(fields),
        },
        EventShape::Attributes => ParsedEvent {
            attributes,
            mapping: None,
        },
        EventShape::Attributed => ParsedEvent {
            attributes,
            mapping: Some(fields),
        },
    };
    state.process_event(event).or_value_error()
}

fn build_attributes<'spec>(
    spec: &'spec EventSpec,
    output: &'spec [OutputItem<'spec>],
) -> EventAttributes<'spec> {
    let mut attributes = EventAttributes::default();
    if let Some(read) = spec.attributes.as_ref() {
        attributes.event_type = read.event_type.as_ref();
        attributes.item = read.item.as_ref();
        attributes.item_id = read.item_id.as_ref();
        attributes.delta = read.delta.as_ref();
        attributes.summary_index = read.summary_index.as_ref();
        attributes.response = match read.response.as_ref() {
            Some(ResponseSpec::Object {
                status,
                usage,
                error,
                ..
            }) => Some(ResponseAttribute::Object(ResponseAttributes {
                output,
                status: status.as_deref(),
                usage: usage.as_ref(),
                error: error.as_ref().map(error_attribute),
            })),
            Some(ResponseSpec::Value { value }) => Some(ResponseAttribute::Value(value)),
            None => None,
        };
    }
    attributes
}

fn error_attribute(spec: &ErrorSpec) -> ErrorAttribute<'_> {
    match spec {
        ErrorSpec::Object {
            message,
            error_type,
            code,
        } => ErrorAttribute::Attributes(StreamErrorAttributes {
            message: message.as_ref(),
            error_type: error_type.as_ref(),
            code: code.as_ref(),
        }),
        ErrorSpec::Value { value } => error_value(value),
    }
}

fn error_value(value: &Json) -> ErrorAttribute<'_> {
    match value {
        Json::Object(fields) => ErrorAttribute::Mapping(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            ErrorAttribute::Attributes(StreamErrorAttributes::default())
        }
    }
}

fn build_redactor(redaction: Redaction) -> Result<Option<Arc<Redactor>>, Box<HelperError>> {
    match redaction {
        Redaction::Enabled => Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map(|redactor| Some(Arc::new(redactor)))
            .or_value_error(),
        Redaction::Disabled => Ok(None),
    }
}
