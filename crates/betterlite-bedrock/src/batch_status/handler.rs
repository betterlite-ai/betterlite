use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::RequestCounts;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use num_traits::cast;
use serde_json::Number;

use crate::batch_status::error::BatchStatusError;
use crate::batch_status::timestamps::epoch_seconds;

// the botocore client the native handler stands in for is always signed with SigV4
pub const SIGNATURE_VERSION: &str = "v4";

#[derive(Debug, Clone, Copy)]
pub enum EpochInput<'value> {
    Json(&'value Json),
    Float(f64),
    DateTime(DateTime<Utc>),
}

pub fn to_epoch(input: Option<EpochInput<'_>>) -> Result<Option<i64>, BatchStatusError> {
    match input {
        None => Ok(None),
        Some(EpochInput::DateTime(moment)) => Ok(Some(epoch_seconds(moment))),
        Some(EpochInput::Float(number)) => truncate(number).map(Some),
        Some(EpochInput::Json(json)) => json_epoch(json),
    }
}

fn json_epoch(value: &Json) -> Result<Option<i64>, BatchStatusError> {
    match value {
        Json::Bool(flag) => Ok(Some(i64::from(*flag))),
        Json::Number(number) => {
            if let Some(seconds) = number.as_i64() {
                return Ok(Some(seconds));
            }
            if number.as_u64().is_some() {
                return Err(BatchStatusError::EpochRange);
            }
            let float = number.as_f64().ok_or(BatchStatusError::EpochRange)?;
            truncate(float).map(Some)
        }
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => Ok(None),
    }
}

fn truncate(number: f64) -> Result<i64, BatchStatusError> {
    if !number.is_finite() {
        return Err(BatchStatusError::NotFinite);
    }
    cast::<f64, i64>(number).ok_or(BatchStatusError::EpochRange)
}

pub fn record_counts_from_response(
    response: &IndexMap<String, Json>,
) -> Result<Option<RequestCounts>, BatchStatusError> {
    let total = integer_field(response.get("totalRecordCount"), "totalRecordCount")?;
    let success = integer_field(response.get("successRecordCount"), "successRecordCount")?;
    let (Some(total_count), Some(completed)) = (total, success) else {
        return Ok(None);
    };
    let failed = integer_field(response.get("errorRecordCount"), "errorRecordCount")?.unwrap_or(0);
    Ok(Some(RequestCounts {
        completed,
        failed,
        total: total_count,
    }))
}

fn integer_field(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<i64>, BatchStatusError> {
    match value {
        Some(Json::Bool(flag)) => Ok(Some(i64::from(*flag))),
        Some(Json::Number(number)) => integer_value(number, field),
        None | Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) => Ok(None),
    }
}

// LiteLLM accepts only Python ints here, so a fraction or exponent reads as missing instead of truncating
fn integer_value(number: &Number, field: &'static str) -> Result<Option<i64>, BatchStatusError> {
    if let Some(count) = number.as_i64() {
        return Ok(Some(count));
    }
    if !number.to_string().contains(['.', 'e', 'E']) {
        // an integer beyond i64 stays an int in Python, which the count record cannot hold
        return Err(BatchStatusError::CountRange { field });
    }
    Ok(None)
}

#[must_use]
pub fn region_from_bedrock_arn(arn: &str) -> Option<String> {
    let mut parts = arn.split(':');
    let _prefix = parts.next()?;
    let _partition = parts.next()?;
    let service = parts.next()?;
    let region = parts.next()?;
    if service != "bedrock" || region.is_empty() {
        return None;
    }
    Some(region.to_owned())
}

#[must_use]
pub fn extract_job_id_from_arn(arn: &str) -> Option<String> {
    if !arn.contains(":model-invocation-job/") {
        return None;
    }
    let job_id = arn.rsplit('/').next()?;
    if job_id.is_empty() {
        return None;
    }
    Some(job_id.to_owned())
}

#[must_use]
pub fn predict_output_file_uri(
    output_prefix: &str,
    input_uri: &str,
    job_id: Option<&str>,
) -> Option<String> {
    let id = job_id.filter(|candidate| !candidate.is_empty())?;
    if output_prefix.is_empty() || input_uri.is_empty() {
        return None;
    }
    let prefix = if output_prefix.ends_with('/') {
        output_prefix.to_owned()
    } else {
        format!("{output_prefix}/")
    };
    let input_basename = input_uri.rsplit('/').next()?;
    if input_basename.is_empty() {
        return None;
    }
    Some(format!("{prefix}{id}/{input_basename}.out"))
}
