use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

use super::credentials::Credentials;
use super::ids::{IdSource, RequiredIds, RunIds, ensure_required_ids};
use super::metadata::{MetadataFields, extract_metadata_fields};
use super::outputs::{OutputError, build_outputs_with_usage};
use super::redaction::{build_extra_metadata, redact_metadata};

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("standard_logging_object is missing from the callback kwargs")]
    MissingStandardLoggingObject,
    #[error("the standard logging payload could not be written as JSON")]
    Payload(#[source] serde_json::Error),
    #[error("the standard logging payload is not a JSON object")]
    PayloadShape,
    #[error(transparent)]
    Output(#[from] OutputError),
}

#[derive(Clone, Copy)]
pub struct Prepare<'event> {
    pub standard_logging_object: Option<&'event StandardLoggingPayload>,
    pub litellm_metadata: &'event IndexMap<String, Json>,
    pub credentials: &'event Credentials,
    pub default_run_name: &'event str,
    pub redact_user_api_key_info: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunData {
    pub name: String,
    pub run_type: &'static str,
    pub inputs: IndexMap<String, Json>,
    pub outputs: IndexMap<String, Json>,
    pub session_name: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub start_time: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub end_time: Decimal,
    pub tags: Vec<Json>,
    pub extra: IndexMap<String, Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub dotted_order: String,
}

struct RunMetadata {
    inputs: IndexMap<String, Json>,
    extra: IndexMap<String, Json>,
}

struct RunParts<'event> {
    payload: &'event StandardLoggingPayload,
    fields: MetadataFields,
    required: RequiredIds,
    metadata: RunMetadata,
    outputs: IndexMap<String, Json>,
}

// LangsmithLogger._prepare_log_data: the run object every LangSmith event queues, without the unused response object.
pub fn prepare_log_data(
    prepare: &Prepare<'_>,
    now: DateTime<Utc>,
    ids: &dyn IdSource,
) -> Result<RunData, PayloadError> {
    let payload = prepare
        .standard_logging_object
        .ok_or(PayloadError::MissingStandardLoggingObject)?;
    let mut fields = extract_metadata_fields(
        prepare.litellm_metadata,
        prepare.credentials,
        prepare.default_run_name,
    );
    // LangSmith rejects the batch when a run body's session id repeats the tracer session in its trace id.
    if fields.session_id == fields.trace_id {
        fields.session_id = None;
    }
    let metadata = run_inputs(payload, prepare.redact_user_api_key_info)?;
    let outputs = build_outputs_with_usage(payload)?;
    let required = resolve_ids(&fields, now, ids);
    Ok(assemble_run(RunParts {
        payload,
        fields,
        required,
        metadata,
        outputs,
    }))
}

fn run_inputs(
    payload: &StandardLoggingPayload,
    redact_user_api_key_info: bool,
) -> Result<RunMetadata, PayloadError> {
    let metadata = json_object(&payload.metadata)?;
    let extra = build_extra_metadata(metadata.clone(), redact_user_api_key_info);
    let mut inputs = json_object(payload)?;
    inputs.insert(
        "metadata".to_owned(),
        Json::Object(redact_metadata(&metadata, redact_user_api_key_info)),
    );
    Ok(RunMetadata { inputs, extra })
}

fn resolve_ids(fields: &MetadataFields, now: DateTime<Utc>, ids: &dyn IdSource) -> RequiredIds {
    let run_ids = RunIds {
        id: fields.run_id.clone(),
        parent_run_id: fields.parent_run_id.clone(),
        trace_id: fields.trace_id.clone(),
        dotted_order: fields.dotted_order.clone(),
    };
    ensure_required_ids(&run_ids, fields.run_id.as_deref(), now, ids)
}

fn assemble_run(parts: RunParts<'_>) -> RunData {
    RunData {
        name: parts.fields.run_name,
        run_type: "llm",
        inputs: parts.metadata.inputs,
        outputs: parts.outputs,
        session_name: parts.fields.project_name,
        start_time: parts.payload.start_time,
        end_time: parts.payload.end_time,
        tags: parts.payload.request_tags.clone(),
        extra: parts.metadata.extra,
        error: parts
            .payload
            .error_str
            .as_ref()
            .filter(|_| parts.payload.status == LoggingStatus::Failure)
            .cloned(),
        id: parts.required.id,
        parent_run_id: parts.fields.parent_run_id,
        trace_id: parts.required.trace_id,
        session_id: parts.fields.session_id,
        dotted_order: parts.required.dotted_order,
    }
}

fn json_object<T>(value: &T) -> Result<IndexMap<String, Json>, PayloadError>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(PayloadError::Payload)?;
    let decoded: Json = serde_json::from_value(encoded).map_err(PayloadError::Payload)?;
    match decoded {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(PayloadError::PayloadShape)
        }
    }
}
