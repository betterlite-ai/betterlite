use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{
    SecretMap, decode_secret_map, encrypt_secret_map,
};
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use secrecy::SecretString;

use super::blob::decode_json_column;
use super::codec::{reencrypt_credentials_blob, reencrypt_user_credential, reencrypt_user_env_var};
use super::env_vars::reencrypt_global_env_var_values;
use super::error::{RotationCodecError, RotationError};
use super::store::{McpRotationStore, ServerUpdate};

pub struct Rotation<'a, S, N, D> {
    pub store: &'a S,
    pub cipher: &'a StoredCipher<N, D>,
    pub new_key: &'a SecretString,
}

impl<S, N, D> Rotation<'_, S, N, D>
where
    S: McpRotationStore,
    N: NonceSource + Sync,
    D: SecretDiagnostics + Sync,
{
    pub async fn servers(&self, touched_by: &str) -> Result<(), RotationError<S::Error>> {
        let rows = self
            .store
            .server_rows()
            .await
            .map_err(RotationError::Store)?;
        let mut updated = 0_usize;
        for row in rows {
            let mut update = ServerUpdate {
                updated_by: touched_by.to_owned(),
                ..ServerUpdate::default()
            };
            let credentials = decode_json_column(row.credentials.as_deref())?;
            update.credentials =
                reencrypt_credentials_blob(credentials.as_ref(), self.cipher, self.new_key)?;
            update.env_vars = reencrypt_global_env_var_values(
                row.env_vars.as_deref(),
                self.cipher,
                self.new_key,
            )?;
            update.static_headers = self.secret_map_column(
                &row.server_id,
                row.static_headers.as_deref(),
                "static_headers",
            )?;
            update.env = self.secret_map_column(&row.server_id, row.env.as_deref(), "env")?;
            if !update.has_changes() {
                continue;
            }
            self.store
                .update_server(&row.server_id, &update)
                .await
                .map_err(RotationError::Store)?;
            updated = updated.saturating_add(1);
        }
        let oauth_clients = self
            .store
            .oauth_client_rows()
            .await
            .map_err(RotationError::Store)?;
        let mut oauth_updated = 0_usize;
        for client in oauth_clients {
            let credentials = decode_json_column(client.credentials.as_deref())?;
            let Some(rotated) =
                reencrypt_credentials_blob(credentials.as_ref(), self.cipher, self.new_key)?
            else {
                continue;
            };
            self.store
                .update_oauth_client(&client.server_id, &rotated)
                .await
                .map_err(RotationError::Store)?;
            oauth_updated = oauth_updated.saturating_add(1);
        }
        tracing::info!(
            "rotate_mcp_server_credentials_master_key: rotated {updated} MCP server row(s) and {oauth_updated} OAuth-client row(s)"
        );
        Ok(())
    }

    pub async fn user_credentials(&self) -> Result<(), RotationError<S::Error>> {
        let rows = self
            .store
            .user_credential_rows()
            .await
            .map_err(RotationError::Store)?;
        let mut rotated = 0_usize;
        let mut skipped = 0_usize;
        for row in rows {
            let rotated_credential =
                match reencrypt_user_credential(&row.credential_b64, self.cipher, self.new_key)? {
                    Some(JsonValue::String(ciphertext)) => Some(ciphertext),
                    Some(_other) | None => None,
                };
            let Some(ciphertext) = rotated_credential else {
                tracing::warn!(
                    "rotate_mcp_user_credentials_master_key: could not decode credential for user_id={} server_id={}, skipping",
                    row.user_id,
                    row.server_id
                );
                skipped = skipped.saturating_add(1);
                continue;
            };
            self.store
                .update_user_credential(&row.user_id, &row.server_id, &ciphertext)
                .await
                .map_err(RotationError::Store)?;
            rotated = rotated.saturating_add(1);
        }
        tracing::info!(
            "rotate_mcp_user_credentials_master_key: rotated {rotated} row(s), skipped {skipped}"
        );
        Ok(())
    }

    pub async fn user_env_vars(&self) -> Result<(), RotationError<S::Error>> {
        let rows = self
            .store
            .user_env_var_rows()
            .await
            .map_err(RotationError::Store)?;
        let mut rotated = 0_usize;
        let mut skipped = 0_usize;
        for row in rows {
            let rotated_env =
                match reencrypt_user_env_var(&row.values_b64, self.cipher, self.new_key)? {
                    Some(JsonValue::String(ciphertext)) => Some(ciphertext),
                    Some(_other) | None => None,
                };
            let Some(ciphertext) = rotated_env else {
                tracing::warn!(
                    "rotate_mcp_user_env_vars_master_key: could not decrypt env vars for user_id={} server_id={}, skipping",
                    row.user_id,
                    row.server_id
                );
                skipped = skipped.saturating_add(1);
                continue;
            };
            self.store
                .update_user_env_var(&row.user_id, &row.server_id, &ciphertext)
                .await
                .map_err(RotationError::Store)?;
            rotated = rotated.saturating_add(1);
        }
        tracing::info!(
            "rotate_mcp_user_env_vars_master_key: rotated {rotated} row(s), skipped {skipped}"
        );
        Ok(())
    }

    fn secret_map_column(
        &self,
        server_id: &str,
        stored: Option<&str>,
        field: &'static str,
    ) -> Result<Option<JsonValue>, RotationError<S::Error>> {
        let Some(raw) = stored else {
            return Ok(None);
        };
        let Some(value) = decode_json_column(Some(raw))? else {
            return Ok(None);
        };
        let text = match value {
            JsonValue::Null => return Ok(None),
            JsonValue::String(text) => text,
            other => serde_json::to_string(&other).map_err(RotationCodecError::Json)?,
        };
        let map = match decode_stored_map(self.cipher, &text, field) {
            Ok(map) => map,
            Err(SecretError::MapDecode { .. }) => {
                tracing::warn!("Cannot rotate MCP {field} for server {server_id}");
                return Ok(None);
            }
            Err(error) => return Err(RotationCodecError::Secret(error).into()),
        };
        let Some(map) = map.filter(|map| !map.is_empty()) else {
            return Ok(None);
        };
        let ciphertext = encrypt_secret_map(self.cipher, &map, Some(self.new_key))
            .map_err(RotationCodecError::Secret)?;
        Ok(Some(JsonValue::String(ciphertext)))
    }
}

// Python reads a stored string with its text branch, so the value is quoted into stored JSON text.
fn decode_stored_map<N, D>(
    cipher: &StoredCipher<N, D>,
    text: &str,
    field: &str,
) -> Result<Option<SecretMap>, SecretError>
where
    D: SecretDiagnostics,
{
    let quoted = serde_json::to_string(text).map_err(|source| SecretError::MapEncode { source })?;
    decode_secret_map(cipher, Some(&quoted), field)
}
