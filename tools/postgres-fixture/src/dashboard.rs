mod insert;
pub mod payload;
mod schema;
mod tests;
mod validate;

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;

use rust_decimal::Decimal;
use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

use crate::seed::FixtureSeed;

use self::payload::JsonPayload;

const MAX_LOGS: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardSeed {
    #[serde(default)]
    pub logs: Vec<SpendLogSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpendLogSeed {
    pub request_id: String,
    pub call_type: String,
    pub start_time: String,
    pub end_time: String,
    pub key_index: usize,
    #[serde(default)]
    pub completion_start_time: Option<String>,
    #[serde(default)]
    pub request_duration_ms: Option<i32>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub end_user: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub cache_hit: Option<String>,
    #[serde(default)]
    pub litellm_call_id: Option<String>,
    #[serde(default)]
    pub spend: Decimal,
    #[serde(default)]
    pub total_tokens: i32,
    #[serde(default)]
    pub prompt_tokens: i32,
    #[serde(default)]
    pub completion_tokens: i32,
    #[serde(default)]
    pub metadata: Option<JsonPayload>,
    #[serde(default)]
    pub request_tags: Option<JsonPayload>,
    #[serde(default)]
    pub messages: Option<JsonPayload>,
    #[serde(default)]
    pub response: Option<JsonPayload>,
    #[serde(default)]
    pub proxy_server_request: Option<JsonPayload>,
}

#[derive(Debug, Error)]
pub enum DashboardFixtureError {
    #[error("dashboard fixture has {count} logs, more than the limit of {limit}")]
    TooManyLogs { count: usize, limit: usize },
    #[error("dashboard log row {row} has key_index {key_index}, but only {keys} keys are seeded")]
    UnknownKey {
        row: usize,
        key_index: usize,
        keys: usize,
    },
    #[error("dashboard log row {row} repeats an earlier request_id")]
    DuplicateRequestId { row: usize },
    #[error("dashboard log row {row} field {field} is {bytes} bytes, over the limit of {limit}")]
    TextTooLong {
        row: usize,
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    #[error(
        "dashboard log row {row} field {field} serializes to {bytes} bytes, over the limit of {limit}"
    )]
    JsonTooLong {
        row: usize,
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    #[error("dashboard log row {row} field {field} cannot be serialized: {source}")]
    JsonUnserializable {
        row: usize,
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("dashboard log payload cannot be serialized: {source}")]
    PayloadUnserializable {
        #[source]
        source: serde_json::Error,
    },
    #[error("dashboard log row {row} counter {field} is {value}, but counters cannot be negative")]
    NegativeCounter {
        row: usize,
        field: &'static str,
        value: i32,
    },
    #[error("dashboard log row {row} has a negative request_duration_ms")]
    NegativeDuration { row: usize },
    #[error("dashboard schema manifest {path} cannot be read: {source}")]
    ManifestRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("dashboard schema manifest is not UTF-8: {source}")]
    ManifestUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("dashboard schema manifest digest {digest} is not the pinned digest")]
    ManifestDigest { digest: String },
    #[error("dashboard schema manifest line {line} is not a sha256 digest and filename")]
    ManifestLine { line: usize },
    #[error("dashboard schema filename {name:?} is outside the pinned sql directory")]
    UnsafeFilename { name: String },
    #[error("dashboard schema file {file} cannot be read: {source}")]
    SqlRead {
        file: String,
        #[source]
        source: io::Error,
    },
    #[error("dashboard schema file {file} digest {digest} is not the pinned digest")]
    SqlDigest { file: String, digest: String },
    #[error("dashboard schema file {file} is not UTF-8: {source}")]
    SqlUtf8 {
        file: String,
        #[source]
        source: Utf8Error,
    },
    #[error("dashboard log row {row} failed to insert")]
    Insert {
        row: usize,
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("dashboard fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
}

pub fn validate(
    dashboard: &DashboardSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardFixtureError> {
    if dashboard.logs.len() > MAX_LOGS {
        return Err(DashboardFixtureError::TooManyLogs {
            count: dashboard.logs.len(),
            limit: MAX_LOGS,
        });
    }
    let mut request_ids = BTreeSet::new();
    for (row, log) in dashboard.logs.iter().enumerate() {
        validate::validate_log(row, log, seed.keys.len())?;
        if !request_ids.insert(log.request_id.as_str()) {
            return Err(DashboardFixtureError::DuplicateRequestId { row });
        }
    }
    Ok(())
}

pub async fn initialize(client: &Client) -> Result<(), DashboardFixtureError> {
    for statement in schema::verified_sql().await? {
        client
            .batch_execute(&statement)
            .await
            .map_err(DashboardFixtureError::Database)?;
    }
    Ok(())
}

pub async fn seed(
    client: &Client,
    dashboard: &DashboardSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardFixtureError> {
    validate(dashboard, seed)?;
    for (row, log) in dashboard.logs.iter().enumerate() {
        insert::insert_log(client, row, log, seed).await?;
    }
    Ok(())
}
