use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{
    BatchFailure, BatchFailures, BatchObject, BatchObjectKind, BatchStatus,
};
use indexmap::IndexMap;

use crate::batch_status::error::BatchStatusError;
use crate::batch_status::timestamps::parse_iso_timestamp;

const SUBMITTED: &str = "Submitted";
const ENDPOINT: &str = "/v1/chat/completions";
const COMPLETION_WINDOW: &str = "24h";
const IN_PROGRESS_STATUSES: [&str; 3] = ["InProgress", "Validating", "Scheduled"];
const COMPLETED_STATUSES: [&str; 2] = ["Completed", "PartiallyCompleted"];
const FAILED_STATUSES: [&str; 1] = ["Failed"];
const CANCELLED_STATUSES: [&str; 1] = ["Stopped"];
const METADATA_FIELDS: [&str; 6] = [
    "jobName",
    "clientRequestToken",
    "modelId",
    "roleArn",
    "timeoutDurationInHours",
    "vpcConfig",
];

#[derive(Clone, Copy)]
enum BrokenConfig {
    EmptyString,
    Missing,
}

impl BrokenConfig {
    // LiteLLM defaults an absent data config to {}, but only a non-dict one leaves output_file_id None
    const fn value(self) -> Option<&'static str> {
        match self {
            Self::EmptyString => Some(""),
            Self::Missing => None,
        }
    }
}

enum NestedConfig<'data> {
    Absent,
    Object(&'data IndexMap<String, Json>),
    Broken,
}

struct ProjectedStatus {
    status: BatchStatus,
    created_at: i64,
    in_progress_at: Option<i64>,
    completed_at: Option<i64>,
    failed_at: Option<i64>,
    cancelled_at: Option<i64>,
    expires_at: Option<i64>,
}

pub fn decode_batch_status(
    response: &[u8],
    status_code: u16,
    now: i64,
) -> Result<BatchObject, BatchStatusError> {
    let parsed: Json =
        serde_json::from_slice(response).map_err(|source| BatchStatusError::Parse { source })?;
    let Json::Object(data) = parsed else {
        return Err(BatchStatusError::ResponseShape);
    };
    let id = job_arn(&data)?;
    let projected = projected_status(&data, now);
    let (input_file_id, output_file_id) = file_configs(&data)?;
    Ok(BatchObject {
        id,
        completion_window: COMPLETION_WINDOW.to_owned(),
        created_at: projected.created_at,
        endpoint: ENDPOINT.to_owned(),
        input_file_id,
        object: BatchObjectKind::Batch,
        status: projected.status,
        cancelled_at: projected.cancelled_at,
        cancelling_at: None,
        completed_at: projected.completed_at,
        error_file_id: None,
        errors: batch_errors(data.get("message"), status_code)?,
        expired_at: None,
        expires_at: projected.expires_at,
        failed_at: projected.failed_at,
        finalizing_at: None,
        in_progress_at: projected.in_progress_at,
        metadata: Some(enriched_metadata(&data)),
        model: None,
        output_file_id,
        request_counts: None,
        usage: None,
        // the retrieve projection has no passthrough fields, and serde_json's map is a disallowed type to name
        provider_fields: <_>::default(),
    })
}

fn job_arn(data: &IndexMap<String, Json>) -> Result<String, BatchStatusError> {
    match data.get("jobArn") {
        None => Ok(String::new()),
        Some(Json::Text(arn)) => Ok(arn.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchStatusError::Field { field: "jobArn" })
        }
    }
}

fn projected_status(data: &IndexMap<String, Json>, now: i64) -> ProjectedStatus {
    let status_name = match data.get("status") {
        None => Some(SUBMITTED),
        Some(Json::Text(name)) => Some(name.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    };
    let submit_time = timestamp_text(data.get("submitTime"));
    let last_modified = timestamp_text(data.get("lastModifiedTime"));
    let end_time = timestamp_text(data.get("endTime"));
    let expiration = timestamp_text(data.get("jobExpirationTime"));
    let created_at = match submit_time.and_then(parse_iso_timestamp) {
        Some(seconds) if seconds != 0 => seconds,
        Some(_) | None => now,
    };
    ProjectedStatus {
        status: status_name.map_or(BatchStatus::Validating, bedrock_status),
        created_at,
        in_progress_at: conditional_timestamp(status_name, &IN_PROGRESS_STATUSES, last_modified),
        completed_at: conditional_timestamp(status_name, &COMPLETED_STATUSES, end_time),
        failed_at: conditional_timestamp(status_name, &FAILED_STATUSES, end_time),
        cancelled_at: conditional_timestamp(status_name, &CANCELLED_STATUSES, end_time),
        expires_at: expiration.and_then(parse_iso_timestamp),
    }
}

fn bedrock_status(name: &str) -> BatchStatus {
    match name {
        "Scheduled" | "InProgress" => BatchStatus::InProgress,
        "PartiallyCompleted" | "Completed" => BatchStatus::Completed,
        "Failed" => BatchStatus::Failed,
        "Stopping" => BatchStatus::Cancelling,
        "Stopped" => BatchStatus::Cancelled,
        "Expired" => BatchStatus::Expired,
        // Submitted, Validating and every unknown name fall back to validating
        _ => BatchStatus::Validating,
    }
}

fn timestamp_text(value: Option<&Json>) -> Option<&str> {
    let Json::Text(text) = value? else {
        return None;
    };
    Some(text)
}

fn conditional_timestamp(status: Option<&str>, names: &[&str], value: Option<&str>) -> Option<i64> {
    if !status.is_some_and(|name| names.contains(&name)) {
        return None;
    }
    value.and_then(parse_iso_timestamp)
}

fn file_configs(
    data: &IndexMap<String, Json>,
) -> Result<(String, Option<String>), BatchStatusError> {
    let input_file_id = match s3_uri(
        data,
        "inputDataConfig",
        "s3InputDataConfig",
        BrokenConfig::EmptyString,
    )? {
        Some(uri) => uri.to_owned(),
        None => return Err(BatchStatusError::Field { field: "s3Uri" }),
    };
    let output_file_id = s3_uri(
        data,
        "outputDataConfig",
        "s3OutputDataConfig",
        BrokenConfig::Missing,
    )?
    .map(str::to_owned);
    Ok((input_file_id, output_file_id))
}

fn nested_config<'data>(data: &'data IndexMap<String, Json>, key: &str) -> NestedConfig<'data> {
    match data.get(key) {
        None => NestedConfig::Absent,
        Some(Json::Object(entries)) => NestedConfig::Object(entries),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            NestedConfig::Broken
        }
    }
}

fn s3_uri<'data>(
    data: &'data IndexMap<String, Json>,
    outer_key: &str,
    inner_key: &str,
    broken: BrokenConfig,
) -> Result<Option<&'data str>, BatchStatusError> {
    let outer = match nested_config(data, outer_key) {
        NestedConfig::Absent => return Ok(Some("")),
        NestedConfig::Broken => return Ok(broken.value()),
        NestedConfig::Object(entries) => entries,
    };
    let inner = match nested_config(outer, inner_key) {
        NestedConfig::Absent => return Ok(Some("")),
        NestedConfig::Broken => return Ok(broken.value()),
        NestedConfig::Object(entries) => entries,
    };
    match inner.get("s3Uri") {
        None => Ok(Some("")),
        Some(Json::Text(uri)) => Ok(Some(uri)),
        Some(Json::Null) => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchStatusError::Field { field: "s3Uri" })
        }
    }
}

fn batch_errors(
    message: Option<&Json>,
    status_code: u16,
) -> Result<Option<BatchFailures>, BatchStatusError> {
    let text = match message {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(entries)) if entries.is_empty() => return Ok(None),
        Some(Json::Text(entries)) => entries,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(BatchStatusError::Field { field: "message" });
        }
    };
    Ok(Some(BatchFailures {
        data: Some(vec![BatchFailure {
            code: Some(status_code.to_string()),
            line: None,
            message: Some(text.clone()),
            param: None,
        }]),
        object: Some("list".to_owned()),
    }))
}

fn enriched_metadata(data: &IndexMap<String, Json>) -> IndexMap<String, String> {
    let mut metadata = IndexMap::new();
    for field in METADATA_FIELDS {
        let Some(value) = data.get(field) else {
            continue;
        };
        let rendered = match value {
            Json::Null => continue,
            Json::Text(text) => text.clone(),
            Json::Bool(flag) => (if *flag { "True" } else { "False" }).to_owned(),
            Json::Number(number) => number.to_string(),
            Json::List(_) | Json::Object(_) => dumps(value),
        };
        metadata.insert(field.to_owned(), rendered);
    }
    metadata
}
