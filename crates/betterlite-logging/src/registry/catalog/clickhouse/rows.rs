use std::io::{Error as IoError, Result as IoResult, Write};

use chrono::{DateTime, SecondsFormat, Utc};
use flate2::Compression;
use flate2::write::GzEncoder;
use rust_decimal::Decimal;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::storage::ClickHouseStorageError;
use crate::trace_types::SpendLogRecord;

// a retried batch keeps its token, so ClickHouse deduplicates the rows a failed insert already took
pub(super) fn content_token(rows: &[SpendLogRecord]) -> Result<String, ClickHouseStorageError> {
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update(serde_json::to_string(row).map_err(ClickHouseStorageError::Encode)?);
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

// counts the encoded bytes before compression, the way LiteLLM's native insert caps a batch
pub(super) struct LimitedWriter<W> {
    pub(super) inner: W,
    pub(super) remaining: usize,
    pub(super) exceeded: bool,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        if buf.len() > self.remaining {
            self.exceeded = true;
            return Err(IoError::other("the encoded rows exceed the insert limit"));
        }
        let written = self.inner.write(buf)?;
        self.remaining = self.remaining.saturating_sub(written);
        Ok(written)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}

pub(super) fn compressed_rows(
    rows: &[SpendLogRecord],
    received_ms: u64,
    limit: usize,
) -> Result<Vec<u8>, ClickHouseStorageError> {
    let mut writer = LimitedWriter {
        inner: GzEncoder::new(Vec::new(), Compression::default()),
        remaining: limit,
        exceeded: false,
    };
    for (index, row) in rows.iter().enumerate() {
        if index > 0
            && let Err(error) = writer.write_all(b"\n")
        {
            return Err(if writer.exceeded {
                ClickHouseStorageError::InsertTooLarge
            } else {
                ClickHouseStorageError::Compress(error)
            });
        }
        if let Err(error) =
            serde_json::to_writer(&mut writer, &InsertRow::from_row(row, received_ms)?)
        {
            return Err(if writer.exceeded {
                ClickHouseStorageError::InsertTooLarge
            } else {
                ClickHouseStorageError::Encode(error)
            });
        }
    }
    writer
        .inner
        .finish()
        .map_err(ClickHouseStorageError::Compress)
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

// LiteLLM sends the millisecond timestamps as RFC 3339 so DateTime64(3) reads them in its own time zone
fn timestamp(milliseconds: i64) -> Result<String, ClickHouseStorageError> {
    DateTime::<Utc>::from_timestamp_millis(milliseconds)
        .map(|moment| moment.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or(ClickHouseStorageError::InvalidRow)
}
