use std::future::Future;
use std::sync::Arc;

use betterlite_redis::config_sync::table_actions::TableActions as _;
use secrecy::SecretString;

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::actions::user_credential_actions;
use crate::mcp::table_access::rows::KeyValues;
use crate::mcp::table_access::store::McpTableStore;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp_user_credentials::decode::{decode_user_credential, parse_oauth_payload};
use crate::mcp_user_credentials::rows::{CredentialRows, text_field};
use crate::mcp_user_credentials::types::CredentialError;
use crate::stored_cipher::DbCipher;

// The bulk delete purge needs on top of CredentialRows: one WHERE user_id = ? AND server_id = ? per enumerated pair.
pub trait PurgeRows: CredentialRows {
    fn delete_credential_pairs(
        &self,
        server_id: &str,
        user_ids: &[String],
    ) -> impl Future<Output = Result<usize, McpTableError>> + Send;
}

impl PurgeRows for McpTableStore<'_> {
    async fn delete_credential_pairs(
        &self,
        server_id: &str,
        user_ids: &[String],
    ) -> Result<usize, McpTableError> {
        let table = user_credential_actions(Some(self.client), Arc::clone(&self.publish))?;
        let mut deleted = 0_usize;
        for user_id in user_ids {
            let filter = KeyValues::from([
                ("user_id", user_id.clone()),
                ("server_id", server_id.to_owned()),
            ]);
            deleted = deleted.saturating_add(table.delete_many(filter).await?);
        }
        Ok(deleted)
    }
}

// LiteLLM's _decode_oauth_payload test: only rows whose payload decodes as oauth2 are purged, so a BYOK key in the same column stays.
pub(super) fn oauth_users(
    rows: &[JsonData],
    cipher: &DbCipher,
) -> Result<Vec<String>, CredentialError> {
    let mut users = Vec::new();
    for row in rows {
        let user_id =
            text_field(row, "user_id").ok_or(CredentialError::MissingField { field: "user_id" })?;
        let stored = text_field(row, "credential_b64").ok_or(CredentialError::MissingField {
            field: "credential_b64",
        })?;
        let decoded = decode_user_credential(cipher, stored);
        if is_oauth_payload(decoded.as_ref()) {
            users.push(user_id.to_owned());
        }
    }
    Ok(users)
}

// A payload that claims oauth2 but mistypes a field still has no BYOK key to keep, so it goes with the batch as LiteLLM's dict path does.
fn is_oauth_payload(decoded: Option<&SecretString>) -> bool {
    match parse_oauth_payload(decoded) {
        Ok(Some(_)) | Err(_) => true,
        Ok(None) => false,
    }
}
