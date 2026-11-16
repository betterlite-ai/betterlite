use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::types::{
    LockedWrite, LockedWriteOutcome, McpTableError, ServerWrite,
};
use chrono::{DateTime, Utc};

use super::json;
use crate::mcp::server_writes::{
    ChildTable, ServerWriteStore, TokenCacheError, TokenCacheInvalidation,
};

#[derive(Default)]
struct State {
    rows: Vec<JsonData>,
    credential_users: BTreeMap<String, Vec<String>>,
    child_deletes: Vec<(ChildTable, String)>,
    hidden_find: bool,
}

// A row store for the cases a live database cannot be made to show: a lost insert race and a
// cleanup that fails after the row is already gone.
pub(crate) struct FakeStore {
    state: Mutex<State>,
    pub fail_create: bool,
    pub fail_credential_users: bool,
    pub fail_children: bool,
    pub hide_first_find: bool,
}

impl FakeStore {
    pub(crate) fn new(rows: Vec<JsonData>) -> Self {
        Self {
            state: Mutex::new(State {
                rows,
                ..State::default()
            }),
            fail_create: false,
            fail_credential_users: false,
            fail_children: false,
            hide_first_find: false,
        }
    }

    pub(crate) fn with_credential_users(mut self, server_id: &str, users: &[&str]) -> Self {
        let users = users.iter().map(|user| (*user).to_owned()).collect();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.credential_users.insert(server_id.to_owned(), users);
        drop(state);
        self
    }

    pub(crate) fn rows(&self) -> Vec<JsonData> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.rows.clone()
    }

    pub(crate) fn child_deletes(&self) -> Vec<(ChildTable, String)> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.child_deletes.clone()
    }

    fn find(&self, server_id: &str) -> Option<JsonData> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.hide_first_find && !state.hidden_find {
            state.hidden_find = true;
            return None;
        }
        state
            .rows
            .iter()
            .find(|row| server_id_of(row) == Some(server_id.to_owned()))
            .cloned()
    }

    fn apply(&self, data: &RowFields, server_id: &str) -> Option<JsonData> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let found = state
            .rows
            .iter_mut()
            .find(|row| server_id_of(row) == Some(server_id.to_owned()))?;
        merge(found, data);
        Some(found.clone())
    }
}

impl ServerWriteStore for FakeStore {
    async fn find_server(&self, server_id: &str) -> Result<Option<JsonData>, McpTableError> {
        Ok(self.find(server_id))
    }

    async fn find_drafts(&self) -> Result<Vec<JsonData>, McpTableError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state
            .rows
            .iter()
            .filter(|row| text(row, "approval_status") == Some("draft"))
            .cloned()
            .collect())
    }

    async fn find_draft_updated_since(
        &self,
        server_id: &str,
        cutoff: DateTime<Utc>,
    ) -> Result<Option<JsonData>, McpTableError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state
            .rows
            .iter()
            .find(|row| {
                server_id_of(row).as_deref() == Some(server_id)
                    && text(row, "approval_status") == Some("draft")
                    && updated_at(row).is_some_and(|stamp| stamp >= cutoff)
            })
            .cloned())
    }

    async fn create_server(&self, data: RowFields) -> Result<JsonData, McpTableError> {
        if self.fail_create {
            return Err(McpTableError::Table(TableError::MissingRow));
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let row = JsonData::Object(object(&data));
        let server_id = server_id_of(&row).unwrap_or_default();
        state
            .rows
            .retain(|existing| server_id_of(existing) != Some(server_id.clone()));
        state.rows.push(row.clone());
        Ok(row)
    }

    async fn update_server(&self, update: ServerUpdate) -> Result<Option<JsonData>, McpTableError> {
        Ok(self.apply(&update.data, &update.server_id))
    }

    async fn write_if_identifier_free(
        &self,
        request: LockedWrite,
    ) -> Result<LockedWriteOutcome, McpTableError> {
        let written = match request.write {
            ServerWrite::Create(data) => {
                let row = JsonData::Object(object(&data));
                let server_id = server_id_of(&row).unwrap_or_default();
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state
                    .rows
                    .retain(|existing| server_id_of(existing) != Some(server_id.clone()));
                state.rows.push(row.clone());
                Some(row)
            }
            ServerWrite::Update(update) => self.apply(&update.data, &update.server_id),
        };
        Ok(LockedWriteOutcome::Written(written))
    }

    async fn delete_server(&self, server_id: &str) -> Result<Option<JsonData>, McpTableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let found = state
            .rows
            .iter()
            .position(|row| server_id_of(row).as_deref() == Some(server_id));
        Ok(found.map(|index| state.rows.remove(index)))
    }

    async fn credential_user_ids(&self, server_id: &str) -> Result<Vec<String>, McpTableError> {
        if self.fail_credential_users {
            return Err(McpTableError::Table(TableError::MissingRow));
        }
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state
            .credential_users
            .get(server_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn delete_children(
        &self,
        table: ChildTable,
        server_id: &str,
    ) -> Result<(), McpTableError> {
        if self.fail_children {
            return Err(McpTableError::Table(TableError::MissingRow));
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.child_deletes.push((table, server_id.to_owned()));
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingInvalidation {
    calls: Mutex<Vec<(String, String)>>,
    failing: Mutex<BTreeSet<String>>,
}

impl RecordingInvalidation {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn fail_user(&self, user_id: &str) {
        self.failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned());
    }

    pub(crate) fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TokenCacheInvalidation for RecordingInvalidation {
    async fn invalidate(&self, user_id: &str, server_id: &str) -> Result<(), TokenCacheError> {
        let failing = self
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(user_id);
        if failing {
            return Err(TokenCacheError {
                message: format!("the cache rejected user {user_id}"),
            });
        }
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), server_id.to_owned()));
        Ok(())
    }
}

fn object(data: &RowFields) -> BTreeMap<String, JsonData> {
    let mut fields = BTreeMap::new();
    for (name, value) in &data.0 {
        let text = serde_json::to_string(value).expect("a field value is JSON");
        let decoded = serde_json::from_str(&text).expect("a field value is JSON");
        fields.insert((*name).to_owned(), decoded);
    }
    fields
}

fn merge(row: &mut JsonData, data: &RowFields) {
    let JsonData::Object(fields) = row else {
        return;
    };
    for (name, value) in object(data) {
        fields.insert(name, value);
    }
}

fn server_id_of(row: &JsonData) -> Option<String> {
    text(row, "server_id").map(str::to_owned)
}

fn text<'row>(row: &'row JsonData, name: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = row else {
        return None;
    };
    match fields.get(name)? {
        JsonData::String(value) => Some(value),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

fn updated_at(row: &JsonData) -> Option<DateTime<Utc>> {
    let stamp = text(row, "updated_at")?;
    DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|aware| aware.with_timezone(&Utc))
}

pub(crate) fn draft_row(server_id: &str, updated_at: &str) -> JsonData {
    json(&format!(
        r#"{{"server_id": "{server_id}", "server_name": "{server_id}", "transport": "sse", "approval_status": "draft", "updated_at": "{updated_at}"}}"#
    ))
}

pub(crate) fn active_row(server_id: &str, updated_at: &str) -> JsonData {
    json(&format!(
        r#"{{"server_id": "{server_id}", "server_name": "{server_id}", "transport": "sse", "approval_status": "active", "updated_at": "{updated_at}"}}"#
    ))
}
