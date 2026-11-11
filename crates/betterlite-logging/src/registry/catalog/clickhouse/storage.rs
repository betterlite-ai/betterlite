use std::io::Error as IoError;
use std::io::Write as _;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use chrono::{DateTime, SecondsFormat, Utc};
use flate2::Compression;
use flate2::write::GzEncoder;
use http::Method;
use http::header::{CONTENT_ENCODING, HeaderMap, HeaderValue};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use url::Url;

use super::schema;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use crate::finops::metering::clock::UtcClock;
use crate::trace_types::SpendLogRecord;

const INSERT_TIMEOUT: Duration = Duration::from_secs(30);
const REPLACED_URL_PARAMS: [&str; 6] = [
    "query",
    "async_insert",
    "async_insert_deduplicate",
    "wait_for_async_insert",
    "input_format_skip_unknown_fields",
    "date_time_input_format",
];

pub(super) struct ClickHouseHttpStorage<Time: UtcClock> {
    client: ProxiedClient,
    database: String,
    url: Url,
    clock: Time,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ClickHouseInitError {
    #[error("database must be a nonempty SQL identifier and retention must be positive")]
    InvalidSchema,
    #[error("invalid ClickHouse HTTP URL")]
    InvalidUrl,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ClickHouseStorageError {
    #[error("invalid ClickHouse insert table")]
    InvalidTable,
    #[error("invalid ClickHouse insert row")]
    InvalidRow,
    #[error("database must be a nonempty SQL identifier and retention must be positive")]
    InvalidSchema,
    #[error("the system clock is before the Unix epoch")]
    Clock,
    #[error("the ClickHouse insert row could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the ClickHouse insert body could not be compressed")]
    Compress(#[source] IoError),
    #[error("ClickHouse insert failed with HTTP status {0}")]
    InsertFailed(u16),
    #[error("ClickHouse schema setup failed with HTTP status {0}")]
    SchemaFailed(u16),
    #[error("ClickHouse query transport failed")]
    Transport(#[source] TransportError),
}

impl<Time: UtcClock> ClickHouseHttpStorage<Time> {
    pub(super) fn new(
        client: ProxiedClient,
        database: &str,
        url: &SecretString,
        clock: Time,
    ) -> Result<Self, ClickHouseInitError> {
        if !identifier(database) {
            return Err(ClickHouseInitError::InvalidSchema);
        }
        let parsed =
            Url::parse(url.expose_secret()).map_err(|_error| ClickHouseInitError::InvalidUrl)?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
            return Err(ClickHouseInitError::InvalidUrl);
        }
        Ok(Self {
            client,
            database: database.to_owned(),
            url: parsed,
            clock,
        })
    }

    fn received_millis(&self) -> Result<u64, ClickHouseStorageError> {
        u64::try_from(self.clock.now().timestamp_millis())
            .map_err(|_error| ClickHouseStorageError::Clock)
    }

    fn insert_request(&self, table: &str, body: Vec<u8>, token: String) -> OutboundRequest {
        let mut url = self.url.clone();
        let kept: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _value)| !REPLACED_URL_PARAMS.contains(&key.as_ref()))
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.query_pairs_mut().clear().extend_pairs(kept);
        let query = format!("INSERT INTO `{}`.{table} FORMAT JSONEachRow", self.database);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        OutboundRequest {
            method: Method::POST,
            url,
            query: vec![
                ("query".to_owned(), query),
                ("insert_deduplication_token".to_owned(), token),
                ("async_insert".to_owned(), "1".to_owned()),
                ("async_insert_deduplicate".to_owned(), "1".to_owned()),
                ("wait_for_async_insert".to_owned(), "1".to_owned()),
                (
                    "input_format_skip_unknown_fields".to_owned(),
                    "0".to_owned(),
                ),
                (
                    "date_time_input_format".to_owned(),
                    "best_effort".to_owned(),
                ),
            ],
            headers,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/octet-stream"),
                data: body,
            },
            deadline: INSERT_TIMEOUT,
        }
    }
}

impl<Time: UtcClock + Send + Sync + 'static> ClickHouseStorage for ClickHouseHttpStorage<Time> {
    type Error = ClickHouseStorageError;

    async fn insert_rows(&self, table: &str, rows: &[SpendLogRecord]) -> Result<(), Self::Error> {
        if rows.is_empty() {
            return Ok(());
        }
        if table != SPEND_LOGS_TABLE {
            return Err(ClickHouseStorageError::InvalidTable);
        }
        let token = content_token(rows)?;
        let body = compressed_rows(rows, self.received_millis()?)?;
        let response = self
            .client
            .send_buffered(self.insert_request(table, body, token))
            .await
            .map_err(ClickHouseStorageError::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(ClickHouseStorageError::InsertFailed(response.status));
        }
        Ok(())
    }

    async fn ensure_schema(
        &self,
        _trace_retention_days: u32,
        spend_log_retention_days: u32,
    ) -> Result<(), Self::Error> {
        if spend_log_retention_days == 0 {
            return Err(ClickHouseStorageError::InvalidSchema);
        }
        for statement in schema::statements(&self.database, spend_log_retention_days) {
            let request = OutboundRequest {
                method: Method::POST,
                url: self.url.clone(),
                query: Vec::new(),
                headers: HeaderMap::new(),
                body: RequestBody::Bytes {
                    content_type: HeaderValue::from_static("text/plain"),
                    data: statement.into_bytes(),
                },
                deadline: INSERT_TIMEOUT,
            };
            let response = self
                .client
                .send_buffered(request)
                .await
                .map_err(ClickHouseStorageError::Transport)?;
            if !(200..300).contains(&response.status) {
                return Err(ClickHouseStorageError::SchemaFailed(response.status));
            }
        }
        Ok(())
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn content_token(rows: &[SpendLogRecord]) -> Result<String, ClickHouseStorageError> {
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update(serde_json::to_string(row).map_err(ClickHouseStorageError::Encode)?);
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn compressed_rows(
    rows: &[SpendLogRecord],
    received_ms: u64,
) -> Result<Vec<u8>, ClickHouseStorageError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            encoder
                .write_all(b"\n")
                .map_err(ClickHouseStorageError::Compress)?;
        }
        serde_json::to_writer(&mut encoder, &InsertRow::from_row(row, received_ms)?)
            .map_err(ClickHouseStorageError::Encode)?;
    }
    encoder.finish().map_err(ClickHouseStorageError::Compress)
}

#[derive(Serialize)]
struct InsertRow<'row> {
    request_id: &'row str,
    response_id: &'row str,
    call_type: &'row str,
    api_key: &'row str,
    key_alias: &'row str,
    team_id: &'row str,
    team_alias: &'row str,
    organization_id: &'row str,
    user: &'row str,
    end_user: &'row str,
    model: &'row str,
    model_group: &'row str,
    model_id: &'row str,
    custom_llm_provider: &'row str,
    api_base: &'row str,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    spend: Decimal,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    start_time: String,
    end_time: String,
    completion_start_time: Option<String>,
    status: &'row str,
    error_str: &'row str,
    cache_hit: bool,
    session_id: &'row str,
    trace_id: &'row str,
    span_id: &'row str,
    request_tags: &'row [String],
    metadata: &'row str,
    messages: &'row str,
    response: &'row str,
    #[serde(rename = "EngineReceivedMs")]
    engine_received_ms: u64,
}

impl<'row> InsertRow<'row> {
    fn from_row(
        row: &'row SpendLogRecord,
        engine_received_ms: u64,
    ) -> Result<Self, ClickHouseStorageError> {
        Ok(Self {
            request_id: &row.request_id,
            response_id: &row.response_id,
            call_type: &row.call_type,
            api_key: &row.api_key,
            key_alias: &row.key_alias,
            team_id: &row.team_id,
            team_alias: &row.team_alias,
            organization_id: &row.organization_id,
            user: &row.user,
            end_user: &row.end_user,
            model: &row.model,
            model_group: &row.model_group,
            model_id: &row.model_id,
            custom_llm_provider: &row.custom_llm_provider,
            api_base: &row.api_base,
            spend: row.spend,
            prompt_tokens: row.prompt_tokens,
            completion_tokens: row.completion_tokens,
            total_tokens: row.total_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_write_tokens: row.cache_write_tokens,
            start_time: timestamp(row.start_time)?,
            end_time: timestamp(row.end_time)?,
            completion_start_time: row.completion_start_time.map(timestamp).transpose()?,
            status: &row.status,
            error_str: &row.error_str,
            cache_hit: row.cache_hit,
            session_id: &row.session_id,
            trace_id: &row.trace_id,
            span_id: &row.span_id,
            request_tags: &row.request_tags,
            metadata: &row.metadata,
            messages: &row.messages,
            response: &row.response,
            engine_received_ms,
        })
    }
}

fn timestamp(milliseconds: i64) -> Result<String, ClickHouseStorageError> {
    DateTime::<Utc>::from_timestamp_millis(milliseconds)
        .map(|moment| moment.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or(ClickHouseStorageError::InvalidRow)
}
