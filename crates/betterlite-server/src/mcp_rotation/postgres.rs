use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use thiserror::Error;

use super::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};

const SERVER_ROWS_SQL: &str = r#"SELECT server_id, credentials::text, env_vars::text, static_headers::text, env::text FROM "LiteLLM_MCPServerTable""#;
const SERVER_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPServerTable" SET credentials = COALESCE(to_jsonb($2::text), credentials), env_vars = COALESCE(to_jsonb($3::text), env_vars), static_headers = COALESCE(to_jsonb($4::text), static_headers), env = COALESCE(to_jsonb($5::text), env), updated_by = $6 WHERE server_id = $1"#;
const OAUTH_ROWS_SQL: &str =
    r#"SELECT server_id, credentials::text FROM "LiteLLM_MCPServerOAuthClient""#;
const OAUTH_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPServerOAuthClient" SET credentials = to_jsonb($2::text) WHERE server_id = $1"#;
const USER_CREDENTIAL_ROWS_SQL: &str =
    r#"SELECT user_id, server_id, credential_b64 FROM "LiteLLM_MCPUserCredentials""#;
const USER_CREDENTIAL_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPUserCredentials" SET credential_b64 = $3 WHERE user_id = $1 AND server_id = $2"#;
const USER_ENV_ROWS_SQL: &str =
    r#"SELECT user_id, server_id, values_b64 FROM "LiteLLM_MCPUserEnvVars""#;
const USER_ENV_UPDATE_SQL: &str =
    r#"UPDATE "LiteLLM_MCPUserEnvVars" SET values_b64 = $3 WHERE user_id = $1 AND server_id = $2"#;

#[derive(Debug, Error)]
pub enum PostgresRotationError {
    #[error("the MCP rotation query failed")]
    Database(#[from] DbError),
    #[error("the rotated MCP value cannot be stored")]
    Json(#[source] serde_json::Error),
}

pub struct PostgresRotationStore {
    pub store: PostgresStore,
}

impl McpRotationStore for PostgresRotationStore {
    type Error = PostgresRotationError;

    async fn server_rows(&self) -> Result<Vec<StoredServerRow>, Self::Error> {
        let client = self.store.session().await?;
        let rows = client.query(SERVER_ROWS_SQL, &[]).await.map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            found.push(StoredServerRow {
                server_id: row.try_get("server_id").map_err(backend)?,
                credentials: row.try_get("credentials").map_err(backend)?,
                env_vars: row.try_get("env_vars").map_err(backend)?,
                static_headers: row.try_get("static_headers").map_err(backend)?,
                env: row.try_get("env").map_err(backend)?,
            });
        }
        Ok(found)
    }

    async fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> Result<(), Self::Error> {
        let credentials = update
            .credentials
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(PostgresRotationError::Json)?;
        let env_vars = update
            .env_vars
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(PostgresRotationError::Json)?;
        let static_headers = update
            .static_headers
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(PostgresRotationError::Json)?;
        let env = update
            .env
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(PostgresRotationError::Json)?;
        let client = self.store.session().await?;
        client
            .execute(
                SERVER_UPDATE_SQL,
                &[
                    &server_id,
                    &credentials,
                    &env_vars,
                    &static_headers,
                    &env,
                    &update.updated_by,
                ],
            )
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn oauth_client_rows(&self) -> Result<Vec<StoredOAuthClientRow>, Self::Error> {
        let client = self.store.session().await?;
        let rows = client.query(OAUTH_ROWS_SQL, &[]).await.map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            found.push(StoredOAuthClientRow {
                server_id: row.try_get("server_id").map_err(backend)?,
                credentials: row.try_get("credentials").map_err(backend)?,
            });
        }
        Ok(found)
    }

    async fn update_oauth_client(
        &self,
        server_id: &str,
        credentials: &JsonValue,
    ) -> Result<(), Self::Error> {
        let credentials =
            serde_json::to_string(credentials).map_err(PostgresRotationError::Json)?;
        let client = self.store.session().await?;
        client
            .execute(OAUTH_UPDATE_SQL, &[&server_id, &credentials])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn user_credential_rows(&self) -> Result<Vec<StoredUserCredentialRow>, Self::Error> {
        let client = self.store.session().await?;
        let rows = client
            .query(USER_CREDENTIAL_ROWS_SQL, &[])
            .await
            .map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            found.push(StoredUserCredentialRow {
                user_id: row.try_get("user_id").map_err(backend)?,
                server_id: row.try_get("server_id").map_err(backend)?,
                credential_b64: row.try_get("credential_b64").map_err(backend)?,
            });
        }
        Ok(found)
    }

    async fn update_user_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> Result<(), Self::Error> {
        let client = self.store.session().await?;
        client
            .execute(
                USER_CREDENTIAL_UPDATE_SQL,
                &[&user_id, &server_id, &credential_b64],
            )
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn user_env_var_rows(&self) -> Result<Vec<StoredUserEnvVarRow>, Self::Error> {
        let client = self.store.session().await?;
        let rows = client
            .query(USER_ENV_ROWS_SQL, &[])
            .await
            .map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            found.push(StoredUserEnvVarRow {
                user_id: row.try_get("user_id").map_err(backend)?,
                server_id: row.try_get("server_id").map_err(backend)?,
                values_b64: row.try_get("values_b64").map_err(backend)?,
            });
        }
        Ok(found)
    }

    async fn update_user_env_var(
        &self,
        user_id: &str,
        server_id: &str,
        values_b64: &str,
    ) -> Result<(), Self::Error> {
        let client = self.store.session().await?;
        client
            .execute(USER_ENV_UPDATE_SQL, &[&user_id, &server_id, &values_b64])
            .await
            .map_err(backend)?;
        Ok(())
    }
}
