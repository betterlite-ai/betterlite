pub mod environment;
pub mod error;
pub mod postgres;
pub mod sink;
pub mod store;

mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::model_secrets::{DEFAULT_USER_ID, PreparedModel, prepare_encrypted_model};
use betterlite_db::resource_rows::ResourceRowsError;
use betterlite_db::resource_rows::credential::CredentialItem as CredentialRow;
use betterlite_db::sso_assertion_rows::AssertionRows;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::sso_assertions::rotation as sso_rotation;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;

use environment::{environment_values, sealed_values};
use error::RotationError;
use sink::ModelTableInvalidation;
use store::RotationStore;

use crate::config_override_support::environment::{EnvironmentBatch, RuntimeEnvironment};
use crate::credentials::encryption::decrypt_credential_values;
use crate::credentials::merge::merged_credential;
use crate::db_row_decrypt::deployments::decrypt_model_rows;
use crate::db_row_decrypt::env::decrypt_and_export_env;
use crate::db_row_decrypt::warnings::Warnings;
use crate::mcp_rotation::rotation::Rotation as McpRotation;
use crate::mcp_rotation::store::McpRotationStore;

pub const ENVIRONMENT_VARIABLES: &str = "environment_variables";

// _rotate_master_key's shared sequence: the configured cipher opens what is stored, the explicit key seals it again, the running cipher never changes.
pub struct Rotation<'run, S, M, A, I> {
    pub store: &'run S,
    pub invalidate: &'run I,
    pub mcp: &'run M,
    pub assertions: &'run A,
    pub cipher: &'run DbCipher,
    pub new_master_key: &'run SecretString,
    pub secrets: &'run SecretLookup,
    pub warnings: &'run Warnings,
    pub runtime: &'run mut RuntimeEnvironment,
}

impl<S, M, A, I> Rotation<'_, S, M, A, I>
where
    S: RotationStore,
    M: McpRotationStore,
    A: AssertionRows + Sync,
    I: ModelTableInvalidation,
{
    pub async fn run(
        &mut self,
        actor: Option<&str>,
    ) -> Result<(), RotationError<S::Error, I::Error>> {
        self.rotate_models(actor).await?;
        self.rotate_environment().await?;
        self.rotate_mcp(actor).await;
        self.rotate_assertions().await;
        self.rotate_credentials(actor).await
    }

    // The model scan is the read whose failure only skips this stage; every later failure stops the rotation with the earlier writes kept.
    async fn rotate_models(
        &self,
        actor: Option<&str>,
    ) -> Result<(), RotationError<S::Error, I::Error>> {
        let rows = match self.store.model_rows().await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    "Failed to read the stored models, skipping their rotation: {error}"
                );
                return Ok(());
            }
        };
        if rows.is_empty() {
            return Ok(());
        }
        let environment = self.runtime.snapshot().values;
        let deployments = decrypt_model_rows(
            &rows,
            self.cipher,
            self.secrets,
            &environment,
            self.warnings,
        )?;
        let mut prepared: Vec<PreparedModel> = Vec::with_capacity(deployments.len());
        for deployment in &deployments {
            prepared.push(prepare_encrypted_model(
                deployment,
                self.cipher,
                self.new_master_key,
                actor,
            )?);
        }
        self.store
            .save_model_params(&prepared)
            .await
            .map_err(RotationError::Store)?;
        self.invalidate
            .model_table_changed()
            .await
            .map_err(RotationError::Invalidation)
    }

    // _decrypt_and_set_db_env_variables runs first and changes the runtime even when the following write fails; a value that does not open is dropped.
    async fn rotate_environment(&mut self) -> Result<(), RotationError<S::Error, I::Error>> {
        let rows = match self.store.config_rows().await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!("Failed to read the stored config, skipping its rotation: {error}");
                return Ok(());
            }
        };
        let Some(row) = rows
            .iter()
            .rev()
            .find(|row| row.param_name == ENVIRONMENT_VARIABLES)
        else {
            return Ok(());
        };
        let Some(values) = environment_values(&row.param_value)? else {
            return Ok(());
        };
        let decrypted = decrypt_and_export_env(
            &values,
            false,
            self.cipher,
            &mut BTreeMap::new(),
            self.warnings,
        );
        if decrypted.is_empty() {
            return Ok(());
        }
        let changes = decrypted
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect();
        self.runtime.apply(&EnvironmentBatch { changes });
        let sealed = sealed_values(&decrypted, self.cipher, self.new_master_key)?;
        self.store
            .save_environment_variables(sealed)
            .await
            .map_err(RotationError::Store)
    }

    // Each MCP store reports its own failure and the rotation continues; their partial commits stay visible.
    async fn rotate_mcp(&self, actor: Option<&str>) {
        let touched_by = actor.unwrap_or(DEFAULT_USER_ID);
        let rotation = McpRotation {
            store: self.mcp,
            cipher: self.cipher,
            new_key: self.new_master_key,
        };
        if let Err(error) = rotation.servers(touched_by).await {
            tracing::warn!("Failed to rotate MCP server credentials: {error}");
        }
        if let Err(error) = rotation.user_credentials().await {
            tracing::warn!("Failed to rotate MCP user credentials: {error}");
        }
        if let Err(error) = rotation.user_env_vars().await {
            tracing::warn!("Failed to rotate MCP user env vars: {error}");
        }
    }

    async fn rotate_assertions(&self) {
        if let Err(error) =
            sso_rotation::rotate_assertions(self.assertions, self.cipher, self.new_master_key).await
        {
            tracing::warn!("Failed to rotate SSO identity assertions: {error}");
        }
    }

    // One credential row that does not decode or does not update is skipped; the actor is written as given, null included.
    async fn rotate_credentials(
        &self,
        actor: Option<&str>,
    ) -> Result<(), RotationError<S::Error, I::Error>> {
        let rows = match self.store.credential_rows().await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    "Failed to read the stored credentials, skipping their rotation: {error}"
                );
                return Ok(());
            }
        };
        if rows.is_empty() {
            return Ok(());
        }
        let scanned = rows.len();
        for stored in rows {
            let row = match stored_credential(stored) {
                Ok(row) => row,
                Err(error) => {
                    tracing::error!("Failed to re-encrypt a stored credential: {error}");
                    continue;
                }
            };
            let name = row.fields.base.credential_name.clone();
            if let Err(error) = self.reencrypt_credential(&row, actor).await {
                tracing::error!("Failed to re-encrypt credential {name}: {error}");
            }
        }
        tracing::debug!("Successfully re-encrypted {scanned} credentials with new master key");
        Ok(())
    }

    async fn reencrypt_credential(
        &self,
        row: &CredentialRow,
        actor: Option<&str>,
    ) -> Result<(), RotationError<S::Error, I::Error>> {
        let decrypted = decrypt_credential_values(self.cipher, row);
        let merged = merged_credential(self.cipher, row, &decrypted, Some(self.new_master_key))?;
        self.store
            .update_credential(&row.fields.base.credential_name, &merged, actor)
            .await
            .map_err(RotationError::Store)
    }
}

// credential_info is nullable in LiteLLM's schema; docs/compatibility.md reads null as the payload's default.
fn stored_credential(stored: JsonData) -> Result<CredentialRow, ResourceRowsError> {
    let mut fields = match stored {
        JsonData::Object(fields) => fields,
        other => return CredentialRow::try_from(other),
    };
    if matches!(fields.get("credential_info"), None | Some(JsonData::Null)) {
        fields.insert(
            "credential_info".to_owned(),
            JsonData::Object(BTreeMap::new()),
        );
    }
    CredentialRow::try_from(JsonData::Object(fields))
}
