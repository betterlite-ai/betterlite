use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_user_credentials::rows::{CredentialRows, CredentialScope, text_field};

// An in-memory stand-in for the Prisma user credential table: composite-key lookups, a column filter, and an upsert that rewrites the row in place
pub(super) struct RowStore {
    rows: Mutex<Vec<JsonData>>,
}

impl RowStore {
    pub(super) fn new(rows: &[JsonData]) -> Self {
        Self {
            rows: Mutex::new(rows.to_vec()),
        }
    }

    pub(super) fn pairs(&self) -> Vec<Vec<String>> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.iter()
            .map(|row| {
                vec![
                    text_field(row, "user_id").unwrap_or_default().to_owned(),
                    text_field(row, "server_id").unwrap_or_default().to_owned(),
                ]
            })
            .collect()
    }

    fn find(&self, user_id: &str, server_id: &str) -> Option<JsonData> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.iter()
            .find(|stored| is_pair(stored, user_id, server_id))
            .cloned()
    }

    fn select(&self, field: &str, value: &str) -> Vec<JsonData> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.iter()
            .filter(|stored| text_field(stored, field) == Some(value))
            .cloned()
            .collect()
    }

    fn upsert(&self, user_id: &str, server_id: &str, credential: &str) {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        for stored in rows.iter_mut() {
            if is_pair(stored, user_id, server_id) {
                set_text(stored, "credential_b64", credential);
                return;
            }
        }
        rows.push(credential_row(user_id, server_id, credential));
    }

    fn remove(&self, user_id: &str, server_id: &str) {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.retain(|stored| !is_pair(stored, user_id, server_id));
    }
}

impl CredentialRows for RowStore {
    fn find_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(Ok(self.find(user_id, server_id)))
    }

    fn find_credentials(
        &self,
        scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        ready(Ok(match scope {
            CredentialScope::User(user_id) => self.select("user_id", user_id),
            CredentialScope::Server(server_id) => self.select("server_id", server_id),
        }))
    }

    fn upsert_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> impl Future<Output = Result<(), McpTableError>> + Send {
        self.upsert(user_id, server_id, credential_b64);
        ready(Ok(()))
    }

    fn delete_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), McpTableError>> + Send {
        self.remove(user_id, server_id);
        ready(Ok(()))
    }
}

fn is_pair(stored: &JsonData, user_id: &str, server_id: &str) -> bool {
    text_field(stored, "user_id") == Some(user_id)
        && text_field(stored, "server_id") == Some(server_id)
}

fn credential_row(user_id: &str, server_id: &str, credential: &str) -> JsonData {
    JsonData::Object(std::collections::BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        (
            "server_id".to_owned(),
            JsonData::String(server_id.to_owned()),
        ),
        (
            "credential_b64".to_owned(),
            JsonData::String(credential.to_owned()),
        ),
        (
            "updated_at".to_owned(),
            JsonData::String("2026-01-01T00:00:00+00:00".to_owned()),
        ),
    ]))
}

fn set_text(stored: &mut JsonData, field: &str, value: &str) {
    let JsonData::Object(fields) = stored else {
        return;
    };
    drop(fields.insert(field.to_owned(), JsonData::String(value.to_owned())));
}
