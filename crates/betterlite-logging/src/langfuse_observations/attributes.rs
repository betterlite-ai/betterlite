pub(crate) mod keys;
mod metadata;
mod wire;

use betterlite_models::decisions::Json;
use opentelemetry::{KeyValue, Value};
use serde::Deserialize;

use super::identity::Timestamp;
use metadata::{flattened_metadata, tags_value};
use wire::{serialized, string_or_none};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationType {
    Generation,
    Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PromptLink {
    pub name: String,
    pub version: i64,
    pub is_fallback: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TraceAttributes<'fields> {
    pub name: Option<&'fields Json>,
    pub user_id: Option<&'fields Json>,
    pub session_id: Option<&'fields Json>,
    pub version: Option<&'fields Json>,
    pub release: Option<&'fields Json>,
    pub tags: Option<&'fields Json>,
    pub metadata: Option<&'fields Json>,
    pub public: Option<bool>,
    pub input: Option<&'fields Json>,
    pub output: Option<&'fields Json>,
}

#[derive(Clone, Copy, Debug)]
pub struct ObservationAttributes<'fields> {
    pub observation_type: ObservationType,
    pub input: Option<&'fields Json>,
    pub output: Option<&'fields Json>,
    pub metadata: Option<&'fields Json>,
    pub level: Option<&'fields Json>,
    pub status_message: Option<&'fields Json>,
    pub version: Option<&'fields Json>,
    pub model: Option<&'fields Json>,
    pub model_parameters: Option<&'fields Json>,
    pub usage_details: Option<&'fields Json>,
    pub cost_details: Option<&'fields Json>,
    pub completion_start_time: Option<Timestamp>,
    pub prompt: Option<&'fields PromptLink>,
}

// litellm.integrations.langfuse.langfuse_sdk.trace_attributes; on the root observation these define the trace
#[must_use]
pub fn trace_attributes(fields: &TraceAttributes<'_>) -> Vec<KeyValue> {
    let mut attributes = Vec::new();
    push_text_attributes(
        &mut attributes,
        &[
            (keys::TRACE_NAME, fields.name),
            (keys::TRACE_USER_ID, fields.user_id),
            (keys::TRACE_SESSION_ID, fields.session_id),
            (keys::VERSION, fields.version),
            (keys::RELEASE, fields.release),
        ],
    );
    if let Some(public) = fields.public {
        attributes.push(KeyValue::new(keys::TRACE_PUBLIC, Value::Bool(public)));
    }
    push_serialized_attributes(
        &mut attributes,
        &[
            (keys::TRACE_INPUT, fields.input),
            (keys::TRACE_OUTPUT, fields.output),
        ],
    );
    if let Some(tags) = tags_value(fields.tags) {
        attributes.push(KeyValue::new(keys::TRACE_TAGS, tags));
    }
    attributes.extend(flattened_metadata(keys::TRACE_METADATA, fields.metadata));
    attributes
}

// litellm.integrations.langfuse.langfuse_sdk.observation_attributes; a fallback prompt is not linked
#[must_use]
pub fn observation_attributes(fields: &ObservationAttributes<'_>) -> Vec<KeyValue> {
    let kind = match fields.observation_type {
        ObservationType::Generation => "generation",
        ObservationType::Span => "span",
    };
    let mut attributes = vec![KeyValue::new(keys::OBSERVATION_TYPE, Value::from(kind))];
    push_text_attributes(
        &mut attributes,
        &[
            (keys::OBSERVATION_LEVEL, fields.level),
            (keys::OBSERVATION_STATUS_MESSAGE, fields.status_message),
            (keys::VERSION, fields.version),
        ],
    );
    push_serialized_attributes(
        &mut attributes,
        &[
            (keys::OBSERVATION_INPUT, fields.input),
            (keys::OBSERVATION_OUTPUT, fields.output),
        ],
    );
    push_text_attributes(&mut attributes, &[(keys::OBSERVATION_MODEL, fields.model)]);
    push_serialized_attributes(
        &mut attributes,
        &[
            (keys::OBSERVATION_MODEL_PARAMETERS, fields.model_parameters),
            (keys::OBSERVATION_USAGE_DETAILS, fields.usage_details),
            (keys::OBSERVATION_COST_DETAILS, fields.cost_details),
        ],
    );
    attributes.extend(
        fields
            .completion_start_time
            .map(|start| KeyValue::new(keys::COMPLETION_START, completion_start_text(start))),
    );
    push_prompt(&mut attributes, fields.prompt);
    attributes.extend(flattened_metadata(
        keys::OBSERVATION_METADATA,
        fields.metadata,
    ));
    attributes
}

fn push_text_attributes(attributes: &mut Vec<KeyValue>, entries: &[(&'static str, Option<&Json>)]) {
    for &(key, field) in entries {
        if let Some(text) = string_or_none(field) {
            attributes.push(KeyValue::new(key, Value::from(text)));
        }
    }
}

fn push_serialized_attributes(
    attributes: &mut Vec<KeyValue>,
    entries: &[(&'static str, Option<&Json>)],
) {
    for &(key, field) in entries {
        if let Some(value) = field.filter(|candidate| !matches!(candidate, Json::Null)) {
            attributes.push(KeyValue::new(key, Value::from(serialized(value))));
        }
    }
}

fn push_prompt(attributes: &mut Vec<KeyValue>, prompt: Option<&PromptLink>) {
    let Some(linked) = prompt.filter(|candidate| !candidate.is_fallback) else {
        return;
    };
    attributes.push(KeyValue::new(
        keys::OBSERVATION_PROMPT_NAME,
        Value::from(linked.name.clone()),
    ));
    attributes.push(KeyValue::new(
        keys::OBSERVATION_PROMPT_VERSION,
        Value::I64(linked.version),
    ));
}

// litellm's _serialize_datetime: a datetime reaches the attribute as the JSON string a reader parses back
fn completion_start_text(value: Timestamp) -> String {
    match value {
        Timestamp::DateTime(datetime) => {
            serde_json::to_string(&datetime.to_rfc3339()).unwrap_or_default()
        }
        Timestamp::Seconds(seconds) => seconds.normalize().to_string(),
    }
}

#[path = "attributes/tests.rs"]
mod tests;
