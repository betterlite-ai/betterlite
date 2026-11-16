use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::types::{
    IdentifierField, LockedWrite, LockedWriteOutcome, McpTableError, ServerWrite,
};
use betterlite_server::mcp::server_writes::{
    ChildTable, ServerWriteStore, TokenCacheError, TokenCacheInvalidation,
};
use chrono::{DateTime, Utc};

// An in-memory stand-in for LiteLLM_MCPServerTable and its three child tables, in table order.
pub(super) struct MemoryStore {
    rows: Mutex<Vec<JsonData>>,
    users: Vec<String>,
    credential_server_id: String,
    deleted_children: Mutex<Vec<&'static str>>,
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
            .clone()
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
        Ok(Some(row.clone()))
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
    async fn find_server(&self, server_id: &str) -> Result<Option<JsonData>, McpTableError> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .find(|row| text(row, "server_id") == Some(server_id))
            .cloned())
    }

    async fn find_drafts(&self) -> Result<Vec<JsonData>, McpTableError> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rows
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
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .find(|row| {
                text(row, "server_id") == Some(server_id)
                    && text(row, "approval_status") == Some("draft")
                    && text(row, "updated_at").is_some_and(|stamp| {
                        DateTime::parse_from_rfc3339(stamp)
                            .is_ok_and(|parsed| parsed.with_timezone(&Utc) >= cutoff)
                    })
            })
            .cloned())
    }

    async fn create_server(&self, data: RowFields) -> Result<JsonData, McpTableError> {
        let row = stored_row(&data)?;
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(row.clone());
        Ok(row)
    }

    async fn update_server(&self, update: ServerUpdate) -> Result<Option<JsonData>, McpTableError> {
        self.update_row(&update)
    }

    async fn write_if_identifier_free(
        &self,
        request: LockedWrite,
    ) -> Result<LockedWriteOutcome, McpTableError> {
        if let Some(conflict) = self.conflict(&request) {
            return Ok(LockedWriteOutcome::Conflict(conflict));
        }
        let written = match request.write {
            ServerWrite::Create(data) => Some(self.create_server(data).await?),
            ServerWrite::Update(update) => self.update_row(&update)?,
        };
        Ok(LockedWriteOutcome::Written(written))
    }

    async fn delete_server(&self, server_id: &str) -> Result<Option<JsonData>, McpTableError> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(index) = rows
            .iter()
            .position(|row| text(row, "server_id") == Some(server_id))
        else {
            return Ok(None);
        };
        Ok(Some(rows.remove(index)))
    }

    async fn credential_user_ids(&self, server_id: &str) -> Result<Vec<String>, McpTableError> {
        if server_id == self.credential_server_id {
            return Ok(self.users.clone());
        }
        Ok(Vec::new())
    }

    async fn delete_children(
        &self,
        table: ChildTable,
        _server_id: &str,
    ) -> Result<(), McpTableError> {
        self.deleted_children
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(table.label());
        Ok(())
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
    async fn invalidate(&self, user_id: &str, server_id: &str) -> Result<(), TokenCacheError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), server_id.to_owned()));
        Ok(())
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
