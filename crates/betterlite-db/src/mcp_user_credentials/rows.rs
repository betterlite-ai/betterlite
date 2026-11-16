use std::future::Future;
use std::sync::Arc;

use betterlite_redis::config_sync::table_actions::TableActions as _;

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::actions::user_credential_actions;
use crate::mcp::table_access::rows::KeyValues;
use crate::mcp::table_access::store::McpTableStore;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp::table_access::user_tables::{
    db_find_user_credential_row, db_find_user_credential_rows, db_upsert_user_credential_row,
};

use super::types::CredentialError;

pub(super) const CREDENTIAL_COLUMN: &str = "credential_b64";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialScope<'scope> {
    User(&'scope str),
    Server(&'scope str),
}

pub trait CredentialRows: Send + Sync {
    fn find_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send;

    fn find_credentials(
        &self,
        scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send;

    fn upsert_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> impl Future<Output = Result<(), McpTableError>> + Send;

    fn delete_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), McpTableError>> + Send;
}

impl CredentialRows for McpTableStore<'_> {
    async fn find_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<JsonData>, McpTableError> {
        db_find_user_credential_row(self, user_id, server_id).await
    }

    async fn find_credentials(
        &self,
        scope: CredentialScope<'_>,
    ) -> Result<Vec<JsonData>, McpTableError> {
        let filter = match scope {
            CredentialScope::User(user_id) => KeyValues::from([("user_id", user_id.to_owned())]),
            CredentialScope::Server(server_id) => {
                KeyValues::from([("server_id", server_id.to_owned())])
            }
        };
        db_find_user_credential_rows(self, Some(&filter)).await
    }

    async fn upsert_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> Result<(), McpTableError> {
        db_upsert_user_credential_row(self, user_id, server_id, credential_b64).await
    }

    async fn delete_credential(&self, user_id: &str, server_id: &str) -> Result<(), McpTableError> {
        let filter = KeyValues::from([
            ("user_id", user_id.to_owned()),
            ("server_id", server_id.to_owned()),
        ]);
        let table = user_credential_actions(Some(self.client), Arc::clone(&self.publish))?;
        drop(table.delete(filter).await?);
        Ok(())
    }
}

#[must_use]
pub fn text_field<'row>(row: &'row JsonData, field: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = row else {
        return None;
    };
    let Some(JsonData::String(text)) = fields.get(field) else {
        return None;
    };
    Some(text.as_str())
}

pub(super) fn stored_text(row: &JsonData) -> Result<&str, CredentialError> {
    text_field(row, CREDENTIAL_COLUMN).ok_or(CredentialError::MissingField {
        field: CREDENTIAL_COLUMN,
    })
}
