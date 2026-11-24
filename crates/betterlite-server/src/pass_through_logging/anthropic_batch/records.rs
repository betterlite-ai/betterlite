use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{
    ASSISTANT_ROLE, BATCH_OBJECT, BatchCreationInput, BatchCreationLogged, BatchJobState,
    BatchLoggingChoice, BatchLoggingKwargs, BatchLoggingMessage, BatchLoggingResponse,
    CHOICE_FINISH_REASON, FAILED_MODEL,
};

pub(super) struct SuccessFields<'record> {
    pub(super) batch_id: &'record str,
    pub(super) model_name: &'record str,
    pub(super) unified_object_id: &'record str,
    pub(super) response_id: String,
}

pub(super) fn success_logged(
    input: &BatchCreationInput<'_>,
    fields: &SuccessFields<'_>,
    call_log: &mut LoggingState,
) -> BatchCreationLogged {
    write_call_log(call_log, fields);
    let result = success_response(input.start_time, fields);
    let kwargs = success_kwargs(input, fields);
    BatchCreationLogged { result, kwargs }
}

fn write_call_log(call_log: &mut LoggingState, fields: &SuccessFields<'_>) {
    fields.model_name.clone_into(&mut call_log.model);
    call_log
        .model_call_details
        .insert("model".to_owned(), Json::Text(fields.model_name.to_owned()));
    call_log
        .model_call_details
        .insert("response_cost".to_owned(), decimal_json(Decimal::ZERO));
    call_log.model_call_details.insert(
        "batch_id".to_owned(),
        Json::Text(fields.batch_id.to_owned()),
    );
}

fn success_response(start_time: DateTime<Utc>, fields: &SuccessFields<'_>) -> BatchLoggingResponse {
    let content = format!(
        "Batch job {} created and is pending. Status will be updated when the batch completes.",
        fields.batch_id
    );
    let message = BatchLoggingMessage {
        role: ASSISTANT_ROLE.to_owned(),
        content,
        tool_calls: None,
        function_call: None,
        provider_specific_fields: success_fields(fields),
    };
    BatchLoggingResponse {
        id: fields.response_id.clone(),
        model: fields.model_name.to_owned(),
        object: BATCH_OBJECT.to_owned(),
        created: start_time.timestamp(),
        choices: vec![BatchLoggingChoice {
            finish_reason: CHOICE_FINISH_REASON.to_owned(),
            index: 0,
            message,
        }],
    }
}

fn success_fields(fields: &SuccessFields<'_>) -> IndexMap<String, Json> {
    let mut provider_fields = IndexMap::new();
    provider_fields.insert(
        "batch_job_id".to_owned(),
        Json::Text(fields.batch_id.to_owned()),
    );
    provider_fields.insert(
        "batch_job_state".to_owned(),
        Json::Text(BatchJobState::InProgress.word().to_owned()),
    );
    provider_fields.insert(
        "unified_object_id".to_owned(),
        Json::Text(fields.unified_object_id.to_owned()),
    );
    provider_fields
}

fn success_kwargs(
    input: &BatchCreationInput<'_>,
    fields: &SuccessFields<'_>,
) -> BatchLoggingKwargs {
    BatchLoggingKwargs {
        response_cost: Decimal::ZERO,
        model: fields.model_name.to_owned(),
        batch_id: Some(fields.batch_id.to_owned()),
        unified_object_id: Some(fields.unified_object_id.to_owned()),
        batch_job_state: BatchJobState::InProgress,
        request_metadata: input.request_metadata.clone(),
    }
}

pub(super) enum CreationFailure {
    Status(u16),
    Error(String),
}

pub(super) fn creation_failed(
    input: &BatchCreationInput<'_>,
    response_id: String,
    failure: CreationFailure,
) -> BatchCreationLogged {
    let (content, provider_fields) = match failure {
        CreationFailure::Status(status_code) => (
            format!("Batch job creation failed. Status: {status_code}"),
            failed_fields("status_code", Json::Number(Number::from(status_code))),
        ),
        CreationFailure::Error(error) => {
            let content = format!("Error creating batch job: {error}");
            (content, failed_fields("error", Json::Text(error)))
        }
    };
    failed_logged(input, response_id, content, provider_fields)
}

fn failed_fields(name: &str, value: Json) -> IndexMap<String, Json> {
    let mut provider_fields = IndexMap::new();
    provider_fields.insert(
        "batch_job_state".to_owned(),
        Json::Text(BatchJobState::Failed.word().to_owned()),
    );
    provider_fields.insert(name.to_owned(), value);
    provider_fields
}

fn failed_logged(
    input: &BatchCreationInput<'_>,
    response_id: String,
    content: String,
    provider_specific_fields: IndexMap<String, Json>,
) -> BatchCreationLogged {
    let message = BatchLoggingMessage {
        role: ASSISTANT_ROLE.to_owned(),
        content,
        tool_calls: None,
        function_call: None,
        provider_specific_fields,
    };
    let result = BatchLoggingResponse {
        id: response_id,
        model: FAILED_MODEL.to_owned(),
        object: BATCH_OBJECT.to_owned(),
        created: input.start_time.timestamp(),
        choices: vec![BatchLoggingChoice {
            finish_reason: CHOICE_FINISH_REASON.to_owned(),
            index: 0,
            message,
        }],
    };
    let kwargs = BatchLoggingKwargs {
        response_cost: Decimal::ZERO,
        model: FAILED_MODEL.to_owned(),
        batch_id: None,
        unified_object_id: None,
        batch_job_state: BatchJobState::Failed,
        request_metadata: input.request_metadata.clone(),
    };
    BatchCreationLogged { result, kwargs }
}
