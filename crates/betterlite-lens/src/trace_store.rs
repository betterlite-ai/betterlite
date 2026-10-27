use std::{io::Error, iter::from_fn, num::TryFromIntError, slice::from_ref};

use self::catalogs::Catalogs;
use rusqlite::{Connection, OptionalExtension, params};
mod catalogs;
use tempfile::TempDir;
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};

use crate::analysis::primitives::evidence_valid;
use crate::models::execution::TracePart;
use crate::models::finding::Evidence;

#[derive(Debug, Error)]
pub enum TraceStoreError {
    #[error("span count exceeds platform limits: {0}")]
    Count(#[from] TryFromIntError),
    #[error("catalog row exceeds 24000 bytes")]
    Oversized,
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored trace part is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("temporary trace directory failed: {0}")]
    Temp(#[from] Error),
    #[error("trace store connection is unavailable")]
    Unavailable,
    #[error("blocking trace store task failed: {0}")]
    BlockingJoin(#[from] JoinError),
}

pub struct TraceStore {
    connection: Option<Connection>,
    directory: Option<TempDir>,
}

fn write_parts(
    connection: &Connection,
    parts: &[TracePart],
    statement: &str,
) -> Result<(), TraceStoreError> {
    for part in parts {
        connection.execute(
            statement,
            params![part.span_id, serde_json::to_string(part)?],
        )?;
    }
    Ok(())
}

const READS_INSERT: &str = "INSERT OR IGNORE INTO reads VALUES (?, ?)";

impl TraceStore {
    pub fn new(connection: Connection) -> Result<Self, TraceStoreError> {
        connection.execute(
            "CREATE TABLE spans (span_id TEXT PRIMARY KEY, body TEXT NOT NULL)",
            [],
        )?;
        connection.execute(
            "CREATE TABLE reads (span_id TEXT, body TEXT, UNIQUE(span_id, body))",
            [],
        )?;
        Ok(Self {
            connection: Some(connection),
            directory: None,
        })
    }
    pub fn add(&self, parts: &[TracePart]) -> Result<(), TraceStoreError> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(TraceStoreError::Unavailable)?;
        parts.iter().try_for_each(|part| {
            connection.execute(
                "INSERT OR REPLACE INTO spans VALUES (?, ?)",
                params![part.span_id, serde_json::to_string(part)?],
            )?;
            Ok::<(), TraceStoreError>(())
        })
    }
    pub fn add_reads(&self, parts: &[TracePart]) -> Result<(), TraceStoreError> {
        let connection = self.connection.as_ref();
        connection.map_or_else(
            || Err(TraceStoreError::Unavailable),
            |stored_connection| write_parts(stored_connection, parts, READS_INSERT),
        )
    }

    pub async fn add_async(&mut self, parts: &[TracePart]) -> Result<(), TraceStoreError> {
        let connection = self.connection.take().ok_or(TraceStoreError::Unavailable)?;
        let parts_copy = parts.to_vec();
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let result = write_parts(
                &connection,
                &parts_copy,
                "INSERT OR REPLACE INTO spans VALUES (?, ?)",
            );
            (connection, result)
        });
        let joined = tasks
            .join_next()
            .await
            .ok_or(TraceStoreError::Unavailable)?;
        let result = joined?;
        self.connection = Some(result.0);
        result.1
    }

    pub fn evidence(&self, evidence: &Evidence) -> Result<Option<TracePart>, TraceStoreError> {
        let mut statement = self
            .connection
            .as_ref()
            .ok_or(TraceStoreError::Unavailable)?
            .prepare("SELECT body FROM spans WHERE span_id=? UNION ALL SELECT body FROM reads WHERE span_id=?")?;
        let rows = statement.query_map(params![evidence.span_id, evidence.span_id], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            let part: TracePart = serde_json::from_str(&row?)?;
            if evidence_valid(evidence, from_ref(&part)) {
                return Ok(Some(part));
            }
        }
        Ok(None)
    }
    pub fn parts(&self) -> impl Iterator<Item = Result<TracePart, TraceStoreError>> + '_ {
        let stored_connection = self.connection.as_ref();
        let mut cursor: Option<String> = None;
        let mut exhausted = false;
        from_fn(move || {
            if exhausted {
                return None;
            }
            let Some(connection) = stored_connection else {
                exhausted = true;
                return Some(Err(TraceStoreError::Unavailable));
            };
            let result = OptionalExtension::optional(connection.query_row(
                "SELECT span_id, body FROM spans WHERE ?1 IS NULL OR span_id > ?1 ORDER BY span_id LIMIT 1",
                params![cursor], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            ));
            match result {
                Ok(Some((id, body))) => {
                    cursor = Some(id);
                    Some(serde_json::from_str(&body).map_err(TraceStoreError::Json))
                }
                Ok(None) => {
                    exhausted = true;
                    None
                }
                Err(error) => {
                    exhausted = true;
                    Some(Err(TraceStoreError::Sqlite(error)))
                }
            }
        })
    }
    pub fn get(&self, span_id: &str) -> Result<Option<TracePart>, TraceStoreError> {
        let body = self.optional_string("SELECT body FROM spans WHERE span_id=?", span_id)?;
        body.map(|value| Ok(serde_json::from_str(&value)?))
            .transpose()
    }
    pub fn previous(&self, span_id: &str) -> Result<String, TraceStoreError> {
        let previous = self.optional_string(
            "SELECT span_id FROM spans WHERE span_id < ? ORDER BY span_id DESC LIMIT 1",
            span_id,
        )?;
        Ok(previous.unwrap_or_default())
    }
    fn optional_string(
        &self,
        statement: &str,
        span_id: &str,
    ) -> Result<Option<String>, TraceStoreError> {
        let value = OptionalExtension::optional(
            self.connection
                .as_ref()
                .ok_or(TraceStoreError::Unavailable)?
                .query_row(statement, params![span_id], |row| row.get(0)),
        )?;
        Ok(value)
    }
    pub fn count(&self) -> Result<usize, TraceStoreError> {
        let count = self
            .connection
            .as_ref()
            .ok_or(TraceStoreError::Unavailable)?
            .query_row("SELECT count(*) FROM spans", [], |row| row.get::<_, i64>(0))?;
        Ok(usize::try_from(count)?)
    }
    pub fn catalogs(
        &self,
        root_count: usize,
    ) -> impl Iterator<Item = Result<Vec<CatalogRow>, TraceStoreError>> + '_ {
        let parts = self.parts();
        Catalogs {
            parts,
            pending: None,
            root_count,
        }
    }
}

#[must_use]
pub fn overview_content(part: &TracePart, root_count: usize) -> String {
    let limit = if part.parent_span_id.is_empty() {
        12_000_usize
            .checked_div(root_count.max(1))
            .unwrap_or(0)
            .clamp(160, 2_000)
    } else {
        160
    };
    let length = part.content.chars().count();
    if length <= limit {
        return part.content.clone();
    }
    let first = limit.checked_div(3).unwrap_or(0);
    let prefix: String = part.content.chars().take(first).collect();
    let suffix: String = part
        .content
        .chars()
        .skip(length.saturating_sub(limit.saturating_mul(2).checked_div(3).unwrap_or(0)))
        .collect();
    format!("{prefix}\n[... preview omitted; read this span for evidence ...]\n{suffix}")
}

pub fn trace_store() -> Result<TraceStore, TraceStoreError> {
    let directory = tempfile::Builder::new().prefix("lens-trace-").tempdir()?;
    let path = directory.path().join("trace.sqlite");
    let connection = Connection::open(&path)?;
    let mut store = TraceStore::new(connection)?;
    store.directory = Some(directory);
    Ok(store)
}

pub type CatalogRow = (String, String, String, String, String);

#[path = "trace_store/tests/overview_content.rs"]
mod overview_content;
mod tests;
