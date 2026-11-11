use std::io::Error as IoError;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{CONTENT_ENCODING, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::rows::{compressed_rows, content_token};
use super::schema;
use crate::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use crate::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use crate::finops::metering::clock::UtcClock;
use crate::trace_types::SpendLogRecord;

const INSERT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_INSERT_BYTES: usize = 64 * 1024 * 1024;
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
    #[error("the encoded ClickHouse insert rows exceed 64 MiB")]
    InsertTooLarge,
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
        let body = compressed_rows(rows, self.received_millis()?, MAX_INSERT_BYTES)?;
        let token = content_token(rows)?;
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
