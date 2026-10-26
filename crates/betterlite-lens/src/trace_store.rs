use std::io::Error;
use std::path::Path;

use rusqlite::{Connection, params};
use tempfile::TempDir;
use thiserror::Error;

use crate::models::execution::TracePart;
use crate::models::finding::Evidence;

#[derive(Debug, Error)]
pub enum TraceStoreError {
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored trace part is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("temporary trace directory failed: {0}")]
    Temp(#[from] Error),
}

pub struct TraceStore {
    connection: Connection,
    directory: Option<TempDir>,
}

impl TraceStore {
    pub fn new(connection: Connection) -> Result<Self, TraceStoreError> {
        let store = Self {
            connection,
            directory: None,
        };
        store.create_tables()?;
        Ok(store)
    }
    fn create_tables(&self) -> Result<(), TraceStoreError> {
        self.connection.execute(
            "CREATE TABLE spans (span_id TEXT PRIMARY KEY, body TEXT NOT NULL)",
            [],
        )?;
        self.connection.execute(
            "CREATE TABLE reads (span_id TEXT, body TEXT, UNIQUE(span_id, body))",
            [],
        )?;
        Ok(())
    }
    pub fn add(&self, parts: &[TracePart]) -> Result<(), TraceStoreError> {
        let statement = "INSERT OR REPLACE INTO spans VALUES (?, ?)";
        self.write_parts(parts, statement)
    }
    pub fn add_reads(&self, parts: &[TracePart]) -> Result<(), TraceStoreError> {
        let statement = "INSERT OR IGNORE INTO reads VALUES (?, ?)";
        self.write_parts(parts, statement)
    }
    fn write_parts(&self, parts: &[TracePart], statement: &str) -> Result<(), TraceStoreError> {
        for part in parts {
            self.connection.execute(
                statement,
                params![part.span_id, serde_json::to_string(part)?],
            )?;
        }
        Ok(())
    }
    pub fn evidence(&self, evidence: &Evidence) -> Result<Option<TracePart>, TraceStoreError> {
        let mut statement = self.connection.prepare("SELECT body FROM spans WHERE span_id=? UNION ALL SELECT body FROM reads WHERE span_id=?")?;
        let rows = statement.query_map(params![evidence.span_id, evidence.span_id], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            let part: TracePart = serde_json::from_str(&row?)?;
            if part.execution_id == evidence.execution_id
                && part
                    .content
                    .split("\n[... content omitted ...]\n")
                    .any(|segment| segment.contains(&evidence.quote))
            {
                return Ok(Some(part));
            }
        }
        Ok(None)
    }
    pub fn parts(&self) -> Result<Vec<TracePart>, TraceStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM spans ORDER BY span_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
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
        let value = self
            .connection
            .query_row(statement, params![span_id], |row| row.get(0))
            .optional()?;
        Ok(value)
    }
    pub fn count(&self) -> Result<usize, TraceStoreError> {
        let count = self
            .connection
            .query_row("SELECT count(*) FROM spans", [], |row| row.get::<_, i64>(0))?
            .try_into()
            .unwrap_or(0);
        Ok(count)
    }
    pub fn catalogs(&self, root_count: usize) -> Result<Vec<Vec<CatalogRow>>, TraceStoreError> {
        let mut pages = Vec::new();
        let mut page = Vec::new();
        let mut size: usize = 0;
        for part in self.parts()? {
            let preview = overview_content(&part, root_count);
            let row = (
                part.span_id,
                part.parent_span_id,
                part.name,
                part.kind,
                preview,
            );
            let width = serde_json::to_vec(&row)?.len();
            if !page.is_empty() && size.saturating_add(width) > 24_000 {
                pages.push(page);
                page = Vec::new();
                size = 0;
            }
            page.push(row);
            size = size.saturating_add(width);
        }
        if !page.is_empty() {
            pages.push(page);
        }
        Ok(pages)
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
    let characters: Vec<char> = part.content.chars().collect();
    if characters.len() <= limit {
        return part.content.clone();
    }
    let first = limit.checked_div(3).unwrap_or(0);
    let prefix: String = characters.iter().take(first).collect();
    let suffix: String = characters
        .iter()
        .skip(
            characters
                .len()
                .saturating_sub(limit.saturating_mul(2).checked_div(3).unwrap_or(0)),
        )
        .collect();
    format!("{prefix}\n[... preview omitted; read this span for evidence ...]\n{suffix}")
}

pub fn trace_store() -> Result<TraceStore, TraceStoreError> {
    let directory = tempfile::Builder::new().prefix("lens-trace-").tempdir()?;
    let path = directory.path().join("trace.sqlite");
    let connection = Connection::open(Path::new(&path))?;
    let mut store = TraceStore::new(connection)?;
    store.directory = Some(directory);
    Ok(store)
}

pub type CatalogRow = (String, String, String, String, String);

trait OptionalRow {
    fn optional(self) -> Result<Option<Self::Output>, rusqlite::Error>
    where
        Self: Sized;
    type Output;
}
impl<T> OptionalRow for Result<T, rusqlite::Error> {
    type Output = T;
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[path = "trace_store/tests/overview_content.rs"]
mod overview_content;
mod tests;
