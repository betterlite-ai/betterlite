use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::mcp_toolset_storage::MCPToolsetStorageError;
use betterlite_config::sections::mcp_toolset_storage::table::{
    Patch, ToolsetFilter, ToolsetRow, ToolsetTable, ToolsetUpdate,
};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use chrono::NaiveDateTime;

pub(crate) struct Store {
    reader: Mutex<Vec<ToolsetRow>>,
    writer: Mutex<Vec<ToolsetRow>>,
    seen: Mutex<Vec<String>>,
    fail: bool,
    timestamp: NaiveDateTime,
}

impl Store {
    pub(crate) fn new(rows: Vec<ToolsetRow>, timestamp: NaiveDateTime, fail: bool) -> Self {
        Self {
            reader: Mutex::new(rows.clone()),
            writer: Mutex::new(rows),
            seen: Mutex::new(Vec::new()),
            fail,
            timestamp,
        }
    }

    pub(crate) fn seen(&self) -> String {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .unwrap_or_default()
    }

    fn find_row<F>(&self, matches: F) -> Option<ToolsetRow>
    where
        F: Fn(&ToolsetRow) -> bool,
    {
        self.reader
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|row| matches(row))
            .cloned()
    }

    fn take_row(&self, toolset_id: &str) -> Option<ToolsetRow> {
        let index = self
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .position(|row| row.toolset_id == toolset_id);
        index.map(|found| {
            self.writer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(found)
        })
    }
}

impl ToolsetTable for Store {
    fn create(
        &self,
        row: ToolsetRow,
    ) -> impl Future<Output = Result<ToolsetRow, MCPToolsetStorageError>> + Send {
        let stored = ToolsetRow {
            created_at: Some(self.timestamp),
            updated_at: Some(self.timestamp),
            ..row
        };
        self.writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(stored.clone());
        ready(Ok(stored))
    }

    fn find_unique(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        ready(Ok(self.find_row(|row| row.toolset_id == toolset_id)))
    }

    fn find_first_by_name(
        &self,
        toolset_name: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        ready(Ok(self.find_row(|row| row.toolset_name == toolset_name)))
    }

    fn find_many(
        &self,
        filter: ToolsetFilter,
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<ToolsetRow>, MCPToolsetStorageError>> + Send {
        let matched = {
            let rows = if use_writer {
                self.writer.lock().unwrap_or_else(PoisonError::into_inner)
            } else {
                self.reader.lock().unwrap_or_else(PoisonError::into_inner)
            };
            let label = if use_writer { "writer" } else { "reader" };
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(label.to_owned());
            match &filter {
                ToolsetFilter::All => rows.clone(),
                ToolsetFilter::Ids(ids) => rows
                    .iter()
                    .filter(|row| ids.contains(&row.toolset_id))
                    .cloned()
                    .collect(),
            }
        };
        if self.fail {
            return ready(Err(MCPToolsetStorageError::Table(TableError::Database(
                DbError::NoDbConnected,
            ))));
        }
        ready(Ok(matched))
    }

    fn update(
        &self,
        toolset_id: &str,
        update: ToolsetUpdate,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        let result = {
            let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
            writer
                .iter_mut()
                .find(|row| row.toolset_id == toolset_id)
                .map(|row| {
                    if let Some(name) = &update.toolset_name {
                        row.toolset_name.clone_from(name);
                    }
                    match &update.description {
                        Patch::Keep => {}
                        Patch::Clear => row.description = None,
                        Patch::Set(text) => row.description = Some(text.clone()),
                    }
                    if let Some(tools) = &update.tools {
                        row.tools = Some(tools.clone());
                    }
                    row.updated_by = Some(update.updated_by.clone());
                    row.updated_at = Some(self.timestamp);
                    row.clone()
                })
        };
        ready(Ok(result))
    }

    fn delete(
        &self,
        toolset_id: &str,
    ) -> impl Future<Output = Result<Option<ToolsetRow>, MCPToolsetStorageError>> + Send {
        ready(Ok(self.take_row(toolset_id)))
    }
}
