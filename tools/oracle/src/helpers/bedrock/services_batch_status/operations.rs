use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::batch_status::decode::decode_batch_status;
use betterlite_bedrock::batch_status::handler::{
    EpochInput, SIGNATURE_VERSION, extract_job_id_from_arn, predict_output_file_uri,
    record_counts_from_response, region_from_bedrock_arn, to_epoch,
};
use betterlite_bedrock::batch_status::request::{RetrieveRequest, retrieve_signed_request};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::request_policy::{Params, RequestValue};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::Input;
use super::fixtures::{CredentialInput, Object, Prediction, Retrieve, TimeInput};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Decode {
            now,
            status_code,
            bodies,
        } => Ok(decoded(*now, *status_code, bodies)),
        Input::RecordCounts { responses } => counted(responses),
        Input::OutputUriPrediction { arns, predictions } => Ok(predicted(arns, predictions)),
        Input::RetrieveSignedRequest {
            credentials,
            requests,
        } => signed(credentials, requests),
        Input::RegionAndTime { arns, times } => Ok(region_and_time(arns, times)),
    }
}

fn decoded(now: i64, status_code: u16, bodies: &[String]) -> Value {
    let values: Vec<Value> = bodies
        .iter()
        .map(|body| {
            decode_batch_status(body.as_bytes(), status_code, now).map_or_else(
                |_| json!({"ok": false}),
                |batch| json!({"ok": true, "batch": projected(&batch)}),
            )
        })
        .collect();
    json!({"values": values})
}

fn projected(batch: &BatchObject) -> Value {
    json!({
        "id": &batch.id,
        "object": &batch.object,
        "endpoint": &batch.endpoint,
        "completion_window": &batch.completion_window,
        "status": &batch.status,
        "created_at": batch.created_at,
        "in_progress_at": batch.in_progress_at,
        "completed_at": batch.completed_at,
        "failed_at": batch.failed_at,
        "cancelled_at": batch.cancelled_at,
        "expires_at": batch.expires_at,
        "finalizing_at": batch.finalizing_at,
        "expired_at": batch.expired_at,
        "cancelling_at": batch.cancelling_at,
        "error_file_id": &batch.error_file_id,
        "input_file_id": &batch.input_file_id,
        "output_file_id": &batch.output_file_id,
        "request_counts": &batch.request_counts,
        "errors": &batch.errors,
        "metadata": &batch.metadata,
        "model": &batch.model,
        "usage": &batch.usage,
    })
}

fn counted(responses: &[Object]) -> HelperResult {
    let mut values = Vec::with_capacity(responses.len());
    for response in responses {
        let found = record_counts_from_response(response).or_value_error()?;
        values.push(found.map_or(Value::Null, |counts| {
            json!({"completed": counts.completed, "failed": counts.failed, "total": counts.total})
        }));
    }
    Ok(json!({"counts": values}))
}

fn predicted(arns: &[String], predictions: &[Prediction]) -> Value {
    let job_ids: Vec<Option<String>> = arns
        .iter()
        .map(|arn| extract_job_id_from_arn(arn))
        .collect();
    let uris: Vec<Option<String>> = predictions
        .iter()
        .map(|entry| {
            predict_output_file_uri(
                &entry.output_prefix,
                &entry.input_uri,
                entry.job_id.as_deref(),
            )
        })
        .collect();
    json!({"job_ids": job_ids, "uris": uris})
}

fn signed(credentials: &CredentialInput, requests: &[Retrieve]) -> HelperResult {
    let runtime = runtime()?;
    let mut values = Vec::with_capacity(requests.len());
    for request in requests {
        let deployment = params_of(&request.litellm_params)?;
        let optional = params_of(&request.optional_params)?;
        let now = epoch(request.now)?;
        let attempt = runtime.block_on(retrieve_signed_request(
            RetrieveRequest {
                batch_id: &request.batch_id,
                litellm_params: &deployment,
                optional_params: &optional,
                now,
            },
            |_settings, _region| {
                ready(Ok::<Credentials, CredentialError>(credentials_of(
                    credentials,
                )))
            },
        ));
        values.push(attempt.map_or_else(|_| json!({"ok": false}), |result| signed_json(&result)));
    }
    Ok(json!({"values": values}))
}

fn signed_json(signed: &AwsSignedRequest) -> Value {
    let headers: BTreeMap<&String, &String> = signed
        .headers
        .iter()
        .map(|(name, value)| (name, value))
        .collect();
    json!({
        "ok": true,
        "method": &signed.method,
        "url": &signed.url,
        "headers": headers,
        "data": Value::Null,
    })
}

fn credentials_of(input: &CredentialInput) -> Credentials {
    Credentials {
        access_key_id: input.access_key_id.clone(),
        secret_access_key: input.secret_access_key.clone(),
        session_token: input.session_token.clone(),
    }
}

fn params_of(entries: &Object) -> Result<Params, Box<HelperError>> {
    entries
        .iter()
        .map(|(name, value)| Ok((name.clone(), request_value(value)?)))
        .collect()
}

fn request_value(value: &Json) -> Result<RequestValue, Box<HelperError>> {
    match value {
        Json::Text(text) => Ok(RequestValue::Text(text.clone())),
        Json::Object(entries) => {
            let mut pairs = IndexMap::with_capacity(entries.len());
            for (name, entry) in entries {
                pairs.insert(name.clone(), request_value(entry)?);
            }
            Ok(RequestValue::Mapping(pairs))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            serde_json::to_string(value)
                .map(RequestValue::Raw)
                .or_value_error()
        }
    }
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("the clock is out of range")
        .or_value_error()
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}

fn region_and_time(arns: &[String], times: &[TimeInput]) -> Value {
    let regions: Vec<Option<String>> = arns
        .iter()
        .map(|arn| region_from_bedrock_arn(arn))
        .collect();
    let epochs: Vec<Value> = times.iter().map(epoch_value).collect();
    json!({
        "regions": regions,
        "epochs": epochs,
        "signature_version": SIGNATURE_VERSION,
    })
}

fn epoch_value(time: &TimeInput) -> Value {
    let result = match time {
        TimeInput::Integer(seconds) => to_epoch(Some(EpochInput::Json(&Json::Number(
            serde_json::Number::from(*seconds),
        )))),
        TimeInput::Float(number) => to_epoch(Some(EpochInput::Float(*number))),
        TimeInput::Boolean(flag) => to_epoch(Some(EpochInput::Json(&Json::Bool(*flag)))),
        TimeInput::Datetime(text) => match datetime(text) {
            Some(moment) => to_epoch(Some(EpochInput::DateTime(moment))),
            None => return json!({"failed": true}),
        },
        TimeInput::Text(text) => to_epoch(Some(EpochInput::Json(&Json::Text(text.clone())))),
        TimeInput::Null(()) => to_epoch(Some(EpochInput::Json(&Json::Null))),
        TimeInput::Nonfinite(kind) => match nonfinite(kind) {
            Some(number) => to_epoch(Some(EpochInput::Float(number))),
            None => return json!({"failed": true}),
        },
    };
    match result {
        Ok(Some(seconds)) => json!(seconds),
        Ok(None) => Value::Null,
        Err(_) => json!({"failed": true}),
    }
}

fn datetime(text: &str) -> Option<DateTime<Utc>> {
    let normalized = text.replace('Z', "+00:00");
    let moment = DateTime::parse_from_rfc3339(&normalized).ok()?;
    Some(moment.with_timezone(&Utc))
}

fn nonfinite(kind: &str) -> Option<f64> {
    match kind {
        "inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" => Some(f64::NAN),
        _ => None,
    }
}
