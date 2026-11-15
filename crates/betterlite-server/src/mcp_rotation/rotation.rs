use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{decode_secret_map, encrypt_secret_map};
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use secrecy::SecretString;

use super::codec::{
    USER_CREDENTIAL_KEY, USER_ENV_VARS_KEY, decode_user_value, reencrypt_credentials_blob,
    reencrypt_user_value,
};
use super::env_vars::reencrypt_global_env_values;
use super::error::{RotationCodecError, RotationError};
use super::store::{McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow};

pub struct Rotation<'rotation, S, N, D> {
    pub store: &'rotation S,
    pub cipher: &'rotation StoredCipher<N, D>,
    pub new_key: &'rotation SecretString,
}

#[derive(Clone, Copy)]
enum UserStage {
    Credential,
    EnvVar,
}

impl<S, N, D> Rotation<'_, S, N, D>
where
    S: McpRotationStore,
    N: NonceSource + Sync,
    D: SecretDiagnostics + Sync,
{
    pub async fn servers(&self, touched_by: &str) -> Result<(), RotationError<S::Error>> {
        let mut updated = 0_usize;
        for row in self.store.server_rows().await? {
            if self.rotate_server_row(&row, touched_by).await? {
                updated = updated.saturating_add(1);
            }
        }
        let mut oauth_updated = 0_usize;
        for client in self.store.oauth_client_rows().await? {
            if self.rotate_oauth_client(&client).await? {
                oauth_updated = oauth_updated.saturating_add(1);
            }
        }
        tracing::info!(
            "rotate_mcp_server_credentials_master_key: rotated {updated} MCP server row(s) and {oauth_updated} OAuth-client row(s)"
        );
        Ok(())
    }

    async fn rotate_server_row(
        &self,
        row: &StoredServerRow,
        touched_by: &str,
    ) -> Result<bool, RotationError<S::Error>> {
        let update = ServerUpdate {
            credentials: reencrypt_credentials_blob(
                row.credentials.as_ref(),
                self.cipher,
                self.new_key,
            )
            .map_err(RotationError::Codec)?,
            env_vars: reencrypt_global_env_values(row.env_vars.as_ref(), self.cipher, self.new_key)
                .map_err(RotationError::Codec)?,
            static_headers: self.secret_map_column(
                &row.server_id,
                row.static_headers.as_ref(),
                "static_headers",
            )?,
            env: self.secret_map_column(&row.server_id, row.env.as_ref(), "env")?,
            updated_by: touched_by.to_owned(),
        };
        if !update.has_changes() {
            return Ok(false);
        }
        self.store.update_server(&row.server_id, &update).await?;
        Ok(true)
    }

    async fn rotate_oauth_client(
        &self,
        client: &StoredOAuthClientRow,
    ) -> Result<bool, RotationError<S::Error>> {
        let Some(rotated) =
            reencrypt_credentials_blob(client.credentials.as_ref(), self.cipher, self.new_key)
                .map_err(RotationError::Codec)?
        else {
            return Ok(false);
        };
        self.store
            .update_oauth_client(&client.server_id, &rotated)
            .await?;
        Ok(true)
    }

    pub async fn user_credentials(&self) -> Result<(), RotationError<S::Error>> {
        let (rotated, skipped) = self.rotate_users(UserStage::Credential).await?;
        tracing::info!(
            "rotate_mcp_user_credentials_master_key: rotated {rotated} row(s), skipped {skipped}"
        );
        Ok(())
    }

    pub async fn user_env_vars(&self) -> Result<(), RotationError<S::Error>> {
        let (rotated, skipped) = self.rotate_users(UserStage::EnvVar).await?;
        tracing::info!(
            "rotate_mcp_user_env_vars_master_key: rotated {rotated} row(s), skipped {skipped}"
        );
        Ok(())
    }

    async fn rotate_users(
        &self,
        stage: UserStage,
    ) -> Result<(usize, usize), RotationError<S::Error>> {
        let rows = match stage {
            UserStage::Credential => self
                .store
                .user_credential_rows()
                .await?
                .into_iter()
                .map(|row| (row.user_id, row.server_id, row.credential_b64))
                .collect::<Vec<_>>(),
            UserStage::EnvVar => self
                .store
                .user_env_var_rows()
                .await?
                .into_iter()
                .map(|row| (row.user_id, row.server_id, row.values_b64))
                .collect::<Vec<_>>(),
        };
        let mut rotated = 0_usize;
        let mut skipped = 0_usize;
        for (user_id, server_id, stored) in rows {
            if self
                .rotate_user_row(stage, &user_id, &server_id, &stored)
                .await?
            {
                rotated = rotated.saturating_add(1);
            } else {
                skipped = skipped.saturating_add(1);
            }
        }
        Ok((rotated, skipped))
    }

    async fn rotate_user_row(
        &self,
        stage: UserStage,
        user_id: &str,
        server_id: &str,
        stored: &str,
    ) -> Result<bool, RotationError<S::Error>> {
        let reencrypted = match stage {
            UserStage::Credential => reencrypt_user_value(
                decode_user_value(stored, USER_CREDENTIAL_KEY, self.cipher, true),
                self.cipher,
                self.new_key,
            )
            .map_err(RotationError::Codec)?,
            UserStage::EnvVar => reencrypt_user_value(
                decode_user_value(stored, USER_ENV_VARS_KEY, self.cipher, false),
                self.cipher,
                self.new_key,
            )
            .map_err(RotationError::Codec)?,
        };
        let Some(JsonValue::String(ciphertext)) = reencrypted else {
            match stage {
                UserStage::Credential => tracing::warn!(
                    "rotate_mcp_user_credentials_master_key: could not decode credential for user_id={user_id} server_id={server_id}, skipping"
                ),
                UserStage::EnvVar => tracing::warn!(
                    "rotate_mcp_user_env_vars_master_key: could not decrypt env vars for user_id={user_id} server_id={server_id}, skipping"
                ),
            }
            return Ok(false);
        };
        match stage {
            UserStage::Credential => {
                self.store
                    .update_user_credential(user_id, server_id, &ciphertext)
                    .await?;
            }
            UserStage::EnvVar => {
                self.store
                    .update_user_env_var(user_id, server_id, &ciphertext)
                    .await?;
            }
        }
        Ok(true)
    }

    fn secret_map_column(
        &self,
        server_id: &str,
        stored: Option<&JsonValue>,
        field: &'static str,
    ) -> Result<Option<JsonValue>, RotationError<S::Error>> {
        let Some(value) = stored else {
            return Ok(None);
        };
        if matches!(value, JsonValue::Null) {
            return Ok(None);
        }
        let quoted = serde_json::to_string(value)
            .map_err(RotationCodecError::Json)
            .map_err(RotationError::Codec)?;
        let map = match decode_secret_map(self.cipher, Some(&quoted), field) {
            Ok(map) => map,
            Err(SecretError::MapDecode { .. }) => {
                tracing::warn!("Cannot rotate MCP {field} for server {server_id}");
                return Ok(None);
            }
            Err(error) => return Err(RotationError::Codec(RotationCodecError::Secret(error))),
        };
        let Some(map) = map.filter(|map| !map.is_empty()) else {
            return Ok(None);
        };
        let ciphertext = encrypt_secret_map(self.cipher, &map, Some(self.new_key))
            .map_err(RotationCodecError::Secret)
            .map_err(RotationError::Codec)?;
        Ok(Some(JsonValue::String(ciphertext)))
    }
}
