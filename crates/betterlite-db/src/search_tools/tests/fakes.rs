use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, TimeDelta, Utc};

use crate::auxiliary_repository::repository::TableError;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::DbReconnect;
use crate::mcp_stored_contracts::JsonValue;
use crate::postgres_store::errors::DbError as NativeDbError;
use crate::search_tools::convert::tool::SearchTool;
use crate::search_tools::record::SearchToolRecord;
use crate::search_tools::registry::SearchToolRegistry;
use crate::search_tools::table_client::{
    CreatedAtOrder, SearchToolCreate, SearchToolTableClient, SearchToolUpdate,
};

pub(crate) const EXPECTED_PARAMS: &str = r#"{"api_base":"https://api.example","api_key":"sk-secret","max_retries":3,"search_provider":"tavily","timeout":2.5}"#;
pub(crate) const PARAMS_TEXT: &str = r#"{"search_provider":"tavily"}"#;
pub(crate) const INFO_TEXT: &str = r#"{"description":"Web search"}"#;
pub(crate) const TOOL_JSON: &str = r#"{"search_tool_name":"web","litellm_params":{"search_provider":"tavily","api_key":"sk-secret","api_base":"https://api.example","timeout":2.5,"max_retries":3},"search_tool_info":{"description":"Web search"}}"#;

pub(crate) fn instant() -> DateTime<Utc> {
    DateTime::from_timestamp_micros(1_772_600_767_123_456).unwrap()
}

pub(crate) fn earlier() -> DateTime<Utc> {
    DateTime::from_timestamp(1_772_000_000, 0).unwrap()
}

pub(crate) fn registry() -> SearchToolRegistry {
    SearchToolRegistry { now: instant }
}

pub(crate) fn tool() -> SearchTool {
    serde_json::from_str(TOOL_JSON).unwrap()
}

pub(crate) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn stored_record(search_tool_id: &str, search_tool_name: &str) -> SearchToolRecord {
    SearchToolRecord {
        search_tool_id: search_tool_id.to_owned(),
        search_tool_name: search_tool_name.to_owned(),
        litellm_params: json(PARAMS_TEXT),
        search_tool_info: Some(json(INFO_TEXT)),
        created_at: earlier(),
        updated_at: instant(),
    }
}

fn row_position(rows: &[SearchToolRecord], search_tool_id: &str) -> Result<usize, TableError> {
    rows.iter()
        .position(|row| row.search_tool_id == search_tool_id)
        .ok_or(TableError::MissingRow)
}

pub(crate) fn transport_failure() -> TableError {
    TableError::Database(NativeDbError::NoDbConnected)
}

#[derive(Default)]
pub(crate) struct FakeTable {
    pub(crate) journal: Mutex<Vec<String>>,
    pub(crate) created: Mutex<Vec<SearchToolCreate>>,
    pub(crate) updated: Mutex<Vec<SearchToolUpdate>>,
    pub(crate) rows: Mutex<Vec<SearchToolRecord>>,
    pub(crate) many: Mutex<VecDeque<Result<Vec<SearchToolRecord>, TableError>>>,
    failures: Mutex<Vec<(&'static str, TableError)>>,
}

impl FakeTable {
    pub(crate) fn with_rows(rows: Vec<SearchToolRecord>) -> Self {
        let table = Self::default();
        table.rows.lock().unwrap().extend(rows);
        table
    }

    pub(crate) fn fail(&self, operation: &'static str, failure: TableError) {
        self.failures.lock().unwrap().push((operation, failure));
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.journal.lock().unwrap().clone()
    }

    fn enter(&self, operation: &'static str, entry: String) -> Result<(), TableError> {
        self.journal.lock().unwrap().push(entry);
        let mut failures = self.failures.lock().unwrap();
        let Some(position) = failures.iter().position(|(name, _)| *name == operation) else {
            return Ok(());
        };
        Err(failures.remove(position).1)
    }

    fn rows_for(
        &self,
        operation: &'static str,
        entry: String,
    ) -> Result<MutexGuard<'_, Vec<SearchToolRecord>>, TableError> {
        self.enter(operation, entry)?;
        Ok(self.rows.lock().unwrap())
    }
}

impl SearchToolTableClient for FakeTable {
    async fn create(&self, data: SearchToolCreate) -> Result<SearchToolRecord, TableError> {
        self.enter("create", "create".to_owned())?;
        let shift = TimeDelta::milliseconds(1);
        let record = SearchToolRecord {
            search_tool_id: "generated-id".to_owned(),
            search_tool_name: data.search_tool_name.clone(),
            litellm_params: json(&data.litellm_params),
            search_tool_info: Some(json(&data.search_tool_info)),
            created_at: data.created_at.checked_add_signed(shift).unwrap(),
            updated_at: data.updated_at.checked_add_signed(shift).unwrap(),
        };
        self.created.lock().unwrap().push(data);
        self.rows.lock().unwrap().push(record.clone());
        Ok(record)
    }

    async fn find_unique(
        &self,
        search_tool_id: &str,
    ) -> Result<Option<SearchToolRecord>, TableError> {
        let rows = self.rows_for("find_unique", format!("find_unique:{search_tool_id}"))?;
        Ok(rows
            .iter()
            .find(|row| row.search_tool_id == search_tool_id)
            .cloned())
    }

    async fn find_many(
        &self,
        order: Option<CreatedAtOrder>,
    ) -> Result<Vec<SearchToolRecord>, TableError> {
        self.journal
            .lock()
            .unwrap()
            .push(format!("find_many:{order:?}"));
        let scripted = self.many.lock().unwrap().pop_front();
        scripted.unwrap_or_else(|| Ok(self.rows.lock().unwrap().clone()))
    }

    async fn update(
        &self,
        search_tool_id: &str,
        data: SearchToolUpdate,
    ) -> Result<SearchToolRecord, TableError> {
        let record = {
            let mut rows = self.rows_for("update", format!("update:{search_tool_id}"))?;
            let index = row_position(&rows, search_tool_id)?;
            let row = &mut rows[index];
            row.search_tool_name.clone_from(&data.search_tool_name);
            row.litellm_params = json(&data.litellm_params);
            row.search_tool_info = Some(json(&data.search_tool_info));
            row.updated_at = data.updated_at;
            row.clone()
        };
        self.updated.lock().unwrap().push(data);
        Ok(record)
    }

    async fn delete(&self, search_tool_id: &str) -> Result<SearchToolRecord, TableError> {
        let mut rows = self.rows_for("delete", format!("delete:{search_tool_id}"))?;
        let index = row_position(&rows, search_tool_id)?;
        Ok(rows.remove(index))
    }
}

pub(crate) struct FakeReconnect {
    pub(crate) reasons: Vec<String>,
    pub(crate) succeeds: bool,
}

impl DbReconnect for FakeReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.reasons.push(reason.to_owned());
        let succeeds = self.succeeds;
        Box::pin(async move { Ok(succeeds) })
    }
}
