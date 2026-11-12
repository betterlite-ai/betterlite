#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObject, RequestCounts};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::batch_status::decode::decode_batch_status;
use crate::batch_status::error::BatchStatusError;
use crate::request_policy::{Params, RequestValue};

pub(crate) const NOW: i64 = 1_700_000_000;
pub(crate) const SIGNING_NOW: u64 = 1_767_225_600;
pub(crate) const SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(crate) const JOB_ARN: &str =
    "arn:aws:bedrock:us-east-1:123456789012:model-invocation-job/job-abc";

pub(crate) fn decode(source: &str) -> Result<BatchObject, BatchStatusError> {
    decode_batch_status(source.as_bytes(), 200, NOW)
}

pub(crate) fn decode_with(source: &str, status_code: u16) -> Result<BatchObject, BatchStatusError> {
    decode_batch_status(source.as_bytes(), status_code, NOW)
}

pub(crate) fn response(pairs: &[(&str, &str)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| {
            let parsed = serde_json::from_str(value).expect("the fixture value is JSON");
            ((*key).to_owned(), parsed)
        })
        .collect()
}

pub(crate) fn params(pairs: &[(&str, &str)]) -> Params {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), RequestValue::Text((*value).to_owned())))
        .collect()
}

pub(crate) fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIDEXAMPLE".to_owned()),
        secret_access_key: SecretString::from(SECRET.to_owned()),
        session_token: None,
    }
}

pub(crate) fn signing_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SIGNING_NOW))
        .unwrap()
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(crate) fn metadata_value(batch: &BatchObject, key: &str) -> Option<String> {
    batch
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(key))
        .cloned()
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture value is JSON")
}

pub(crate) fn counts(completed: i64, failed: i64, total: i64) -> RequestCounts {
    RequestCounts {
        completed,
        failed,
        total,
    }
}
