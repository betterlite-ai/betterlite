use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::types::{
    IdentifierField, LockedWrite, LockedWriteOutcome, McpTableError, ServerWrite,
};
use betterlite_server::mcp::outbound_credentials::iso_to_epoch;
use betterlite_server::mcp::server_writes::{
    ChildTable, ServerWriteStore, TokenCacheError, TokenCacheInvalidation,
};
use chrono::{DateTime, Utc};

// An in-memory stand-in for LiteLLM_MCPServerTable and its three child tables, in table order.
pub(super) struct MemoryStore {
    rows: Mutex<Vec<JsonData>>,
    users: Vec<String>,
    credential_server_id: String,
    deleted_children: Mutex<Vec<(&'static str, String)>>,
}

impl MemoryStore {
    pub(super) fn new(rows: &[JsonData], users: &[String], credential_server_id: &str) -> Self {
        Self {
            rows: Mutex::new(rows.to_vec()),
            users: users.to_vec(),
            credential_server_id: credential_server_id.to_owned(),
            deleted_children: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn rows(&self) -> Vec<JsonData> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn deleted_children(&self) -> Vec<&'static str> {
        self.deleted_children
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(label, _server_id)| *label)
            .collect()
    }

    fn matching<F>(&self, keep: F) -> Vec<JsonData>
    where
        F: Fn(&JsonData) -> bool,
    {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|row| keep(row))
            .cloned()
            .collect()
    }

    fn insert(&self, data: &RowFields) -> Result<JsonData, McpTableError> {
        let row = stored_row(data)?;
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(row.clone());
        Ok(row)
    }

    fn live_draft(&self, server_id: &str, cutoff: DateTime<Utc>) -> Option<JsonData> {
        let drafts = self.matching(|row| text(row, "approval_status") == Some("draft"));
        drafts.into_iter().find(|row| {
            text(row, "server_id") == Some(server_id)
                && text(row, "updated_at")
                    .and_then(iso_to_epoch)
                    .is_some_and(|stamp| stamp >= cutoff)
        })
    }

    fn remove(&self, server_id: &str) -> Option<JsonData> {
        let removed = self.matching(|row| text(row, "server_id") == Some(server_id));
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|row| text(row, "server_id") != Some(server_id));
        removed.into_iter().next()
    }

    fn locked_write(&self, request: LockedWrite) -> Result<LockedWriteOutcome, McpTableError> {
        if let Some(conflict) = self.conflict(&request) {
            return Ok(LockedWriteOutcome::Conflict(conflict));
        }
        let written = match request.write {
            ServerWrite::Create(data) => Some(self.insert(&data)?),
            ServerWrite::Update(update) => self.update_row(&update)?,
        };
        Ok(LockedWriteOutcome::Written(written))
    }

    fn update_row(&self, update: &ServerUpdate) -> Result<Option<JsonData>, McpTableError> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(row) = rows
            .iter_mut()
            .find(|row| text(row, "server_id") == Some(update.server_id.as_str()))
        else {
            return Ok(None);
        };
        apply(row, &update.data)?;
        let updated = row.clone();
        drop(rows);
        Ok(Some(updated))
    }

    // Mirrors LiteLLM's _find_mcp_server_identifier_conflict: alias first, both columns, drafts never collide.
    fn conflict(&self, request: &LockedWrite) -> Option<McpIdentifierConflict> {
        let candidates = [
            (IdentifierField::Alias, request.alias.as_deref()),
            (IdentifierField::ServerName, request.server_name.as_deref()),
        ];
        for (field, candidate) in candidates {
            let Some(value) = candidate.filter(|name| !name.is_empty()) else {
                continue;
            };
            let wanted = value.to_lowercase();
            let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let found = rows.iter().find_map(|row| {
                let named = ["server_name", "alias"].iter().any(|column| {
                    text(row, column).is_some_and(|name| name.to_lowercase() == wanted)
                });
                let excluded = request
                    .exclude_server_id
                    .as_deref()
                    .is_some_and(|own| text(row, "server_id") == Some(own));
                if named && !excluded && text(row, "approval_status") != Some("draft") {
                    text(row, "server_id").map(str::to_owned)
                } else {
                    None
                }
            });
            drop(rows);
            if let Some(server_id) = found {
                return Some(McpIdentifierConflict {
                    field,
                    value: value.to_owned(),
                    server_id,
                });
            }
        }
        None
    }
}

impl ServerWriteStore for MemoryStore {
    fn find_server(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        let found = self.matching(|row| text(row, "server_id") == Some(server_id));
        ready(Ok(found.into_iter().next()))
    }

    fn find_drafts(&self) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        ready(Ok(self.matching(|row| {
            text(row, "approval_status") == Some("draft")
        })))
    }

    fn find_draft_updated_since(
        &self,
        server_id: &str,
        cutoff: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(Ok(self.live_draft(server_id, cutoff)))
    }

    fn create_server(
        &self,
        data: RowFields,
    ) -> impl Future<Output = Result<JsonData, McpTableError>> + Send {
        ready(self.insert(&data))
    }

    fn update_server(
        &self,
        update: ServerUpdate,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(self.update_row(&update))
    }

    fn write_if_identifier_free(
        &self,
        request: LockedWrite,
    ) -> impl Future<Output = Result<LockedWriteOutcome, McpTableError>> + Send {
        ready(self.locked_write(request))
    }

    fn delete_server(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(Ok(self.remove(server_id)))
    }

    fn credential_user_ids(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Vec<String>, McpTableError>> + Send {
        let enumerated = (server_id == self.credential_server_id).then(|| self.users.clone());
        ready(Ok(enumerated.unwrap_or_default()))
    }

    fn delete_children(
        &self,
        table: ChildTable,
        server_id: &str,
    ) -> impl Future<Output = Result<(), McpTableError>> + Send {
        let mut deleted = self
            .deleted_children
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        deleted.push((table.label(), server_id.to_owned()));
        drop(deleted);
        ready(Ok(()))
    }
}

// The per-user token cache delete_mcp_server calls; the fake records what the production one would drop.
pub(super) struct RecordingInvalidation {
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingInvalidation {
    pub(super) const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(user_id, _server_id)| user_id.clone())
            .collect()
    }
}

impl TokenCacheInvalidation for RecordingInvalidation {
    fn invalidate(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), TokenCacheError>> + Send {
        let call = (user_id.to_owned(), server_id.to_owned());
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        ready(Ok(()))
    }
}

// The database stamps updated_at; the draft TTL is measured against it, so the fake does too.
fn stored_row(data: &RowFields) -> Result<JsonData, McpTableError> {
    let stamp = JsonData::String(Utc::now().to_rfc3339());
    let mut row = JsonData::Object(BTreeMap::from([("updated_at".to_owned(), stamp)]));
    apply(&mut row, data)?;
    Ok(row)
}

fn apply(stored: &mut JsonData, data: &RowFields) -> Result<(), McpTableError> {
    let JsonData::Object(fields) = stored else {
        return Ok(());
    };
    for (name, value) in &data.0 {
        let text = serde_json::to_string(value).map_err(McpTableError::Json)?;
        let parsed = serde_json::from_str(&text).map_err(McpTableError::Json)?;
        fields.insert((*name).to_owned(), parsed);
    }
    Ok(())
}

fn text<'row>(stored: &'row JsonData, name: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = stored else {
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
