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

// The one failure a fake store plays out, so each test names the situation it needs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    Nothing,
    // The first read misses the row and the insert fails, as for the loser of a create race.
    LostCreateRace,
    CredentialEnumeration,
    ChildCleanup,
}

struct State {
    rows: Vec<JsonData>,
    credential_users: BTreeMap<String, Vec<String>>,
    child_deletes: Vec<(ChildTable, String)>,
    hidden_find: bool,
}

// A row store for what a live database cannot show: a lost insert race and a cleanup that fails late.
pub(crate) struct FakeStore {
    state: Mutex<State>,
    pub(crate) fault: Fault,
}

impl FakeStore {
    pub(crate) const fn new(rows: Vec<JsonData>) -> Self {
        Self {
            state: Mutex::new(State {
                rows,
                credential_users: BTreeMap::new(),
                child_deletes: Vec::new(),
                hidden_find: false,
            }),
            fault: Fault::Nothing,
        }
    }

    pub(crate) fn with_credential_users(self, server_id: &str, users: &[&str]) -> Self {
        let owned = users.iter().map(|user| (*user).to_owned()).collect();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.credential_users.insert(server_id.to_owned(), owned);
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
        if self.fault == Fault::LostCreateRace && !state.hidden_find {
            state.hidden_find = true;
            return None;
        }
        state
            .rows
            .iter()
            .find(|row| server_id_of(row).as_deref() == Some(server_id))
            .cloned()
    }

    fn apply(&self, data: &RowFields, server_id: &str) -> Option<JsonData> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let found = state
            .rows
            .iter_mut()
            .find(|row| server_id_of(row).as_deref() == Some(server_id))?;
        if let JsonData::Object(fields) = found {
            fields.extend(object(data));
        }
        Some(found.clone())
    }

    fn insert(&self, data: &RowFields) -> JsonData {
        let row = JsonData::Object(object(data));
        let server_id = server_id_of(&row);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .rows
            .retain(|existing| server_id_of(existing) != server_id);
        state.rows.push(row.clone());
        drop(state);
        row
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
                    && text(row, "updated_at").is_some_and(|stamp| {
                        DateTime::parse_from_rfc3339(stamp)
                            .is_ok_and(|aware| aware.with_timezone(&Utc) >= cutoff)
                    })
            })
            .cloned())
    }

    async fn create_server(&self, data: RowFields) -> Result<JsonData, McpTableError> {
        if self.fault == Fault::LostCreateRace {
            return Err(McpTableError::Table(TableError::MissingRow));
        }
        Ok(self.insert(&data))
    }

    async fn update_server(&self, update: ServerUpdate) -> Result<Option<JsonData>, McpTableError> {
        Ok(self.apply(&update.data, &update.server_id))
    }

    async fn write_if_identifier_free(
        &self,
        request: LockedWrite,
    ) -> Result<LockedWriteOutcome, McpTableError> {
        let written = match request.write {
            ServerWrite::Create(data) => Some(self.insert(&data)),
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
        if self.fault == Fault::CredentialEnumeration {
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
        if self.fault == Fault::ChildCleanup {
            return Err(McpTableError::Table(TableError::MissingRow));
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .child_deletes
            .push((table, server_id.to_owned()));
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
        let encoded = serde_json::to_string(value).expect("a field value is JSON");
        let decoded = serde_json::from_str(&encoded).expect("a field value is JSON");
        fields.insert((*name).to_owned(), decoded);
    }
    fields
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
