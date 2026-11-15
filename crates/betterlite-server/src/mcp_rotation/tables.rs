use std::convert::Infallible;

use betterlite_db::auxiliary_repository::bindings::{
    MCPServerOAuthClient, MCPServerTable, MCPUserCredentials,
};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableAccess, TableBinding, TableError,
};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions;
use thiserror::Error;
use tokio_postgres::Error as DriverError;
use tokio_postgres::types::ToSql;

use super::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};

// Each table is read whole, as find_many with no where does.
const SERVER_ROWS_SQL: &str = r#"SELECT server_id, credentials::text, env_vars::text, static_headers::text, env::text FROM "LiteLLM_MCPServerTable""#;
const OAUTH_ROWS_SQL: &str =
    r#"SELECT server_id, credentials::text FROM "LiteLLM_MCPServerOAuthClient""#;
const USER_CREDENTIAL_ROWS_SQL: &str =
    r#"SELECT user_id, server_id, credential_b64 FROM "LiteLLM_MCPUserCredentials""#;
const USER_ENV_VAR_ROWS_SQL: &str =
    r#"SELECT user_id, server_id, values_b64 FROM "LiteLLM_MCPUserEnvVars""#;

// A column that needs no change is NULL here and keeps its stored value. Prisma stamps updated_at on
// every update, so each statement does too. A text value is cast to jsonb from its JSON encoding,
// which stores the serialized blob as a JSON string the way Prisma does for a Python str.
const SERVER_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPServerTable" SET credentials = COALESCE($2::text::jsonb, credentials), env_vars = COALESCE($3::text::jsonb, env_vars), static_headers = COALESCE($4::text::jsonb, static_headers), env = COALESCE($5::text::jsonb, env), updated_by = $6, updated_at = now() WHERE server_id = $1"#;
const OAUTH_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPServerOAuthClient" SET credentials = $2::text::jsonb, updated_at = now() WHERE server_id = $1"#;
const USER_CREDENTIAL_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPUserCredentials" SET credential_b64 = $3, updated_at = now() WHERE user_id = $1 AND server_id = $2"#;
const USER_ENV_VAR_UPDATE_SQL: &str = r#"UPDATE "LiteLLM_MCPUserEnvVars" SET values_b64 = $3, updated_at = now() WHERE user_id = $1 AND server_id = $2"#;

// B726 has no binding for this table; LiteLLM names it in _MCPUserEnvVarsRepository.
pub struct McpUserEnvVars;

impl TableBinding for McpUserEnvVars {
    const NAME: &'static str = "litellm_mcpuserenvvars";
    const TABLE: &'static str = "LiteLLM_MCPUserEnvVars";
}

#[derive(Debug, Error)]
pub enum PostgresRotationError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error("the rotated MCP value cannot be stored")]
    Json(#[from] serde_json::Error),
}

// The rotation only updates rows, so every other table action reports that it has nothing to do.
pub struct McpUpdateActions<'db> {
    pub access: TableAccess<'db>,
    pub update_sql: &'static str,
}

impl McpUpdateActions<'_> {
    async fn changed(&self, data: &[Option<String>], keys: &[String]) -> Result<usize, TableError> {
        let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(keys.len() + data.len());
        params.extend(keys.iter().map(|key| key as &(dyn ToSql + Sync)));
        params.extend(data.iter().map(|value| value as &(dyn ToSql + Sync)));
        let client = self.access.write.session().await?;
        let rows = client
            .execute(self.update_sql, &params)
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        usize::try_from(rows).map_err(|source| TableError::RowCount { source })
    }
}

impl TableActions for McpUpdateActions<'_> {
    type Row = ();
    type Filter = Vec<String>;
    type CreateData = ();
    type UpdateData = Vec<Option<String>>;
    type Failure = TableError;

    async fn create(&self, _data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        Err(TableError::MissingRow)
    }

    async fn create_many(&self, _data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        Err(TableError::MissingRow)
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        Ok((self.changed(&data, &filter).await? > 0).then_some(()))
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        self.changed(&data, &filter).await
    }

    async fn upsert(
        &self,
        _filter: Self::Filter,
        _data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        Err(TableError::MissingRow)
    }

    async fn delete(&self, _filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        Err(TableError::MissingRow)
    }

    async fn delete_many(&self, _filter: Self::Filter) -> Result<usize, Self::Failure> {
        Err(TableError::MissingRow)
    }
}

pub struct PostgresRotationStore<'db> {
    servers: PrismaTableRepository<'db, MCPServerTable>,
    oauth_clients: PrismaTableRepository<'db, MCPServerOAuthClient>,
    user_credentials: PrismaTableRepository<'db, MCPUserCredentials>,
    user_env_vars: PrismaTableRepository<'db, McpUserEnvVars>,
}

impl<'db> PostgresRotationStore<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: &PublishChange<TableError>) -> Self {
        Self {
            servers: PrismaTableRepository::new(client, publish.clone(), MCPServerTable),
            oauth_clients: PrismaTableRepository::new(
                client,
                publish.clone(),
                MCPServerOAuthClient,
            ),
            user_credentials: PrismaTableRepository::new(
                client,
                publish.clone(),
                MCPUserCredentials,
            ),
            user_env_vars: PrismaTableRepository::new(client, publish.clone(), McpUserEnvVars),
        }
    }
}

impl McpRotationStore for PostgresRotationStore<'_> {
    type Error = PostgresRotationError;

    async fn server_rows(&self) -> Result<Vec<StoredServerRow>, Self::Error> {
        let client = self.servers.access()?.read.session().await?;
        let rows = client
            .query(SERVER_ROWS_SQL, &[])
            .await
            .map_err(backend::<Infallible>)?;
        let found = rows
            .iter()
            .map(|row| {
                Ok(StoredServerRow {
                    server_id: row.try_get("server_id")?,
                    credentials: row.try_get("credentials")?,
                    env_vars: row.try_get("env_vars")?,
                    static_headers: row.try_get("static_headers")?,
                    env: row.try_get("env")?,
                })
            })
            .collect::<Result<Vec<_>, DriverError>>()
            .map_err(backend::<Infallible>)?;
        Ok(found)
    }

    async fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> Result<(), Self::Error> {
        let mut data = Vec::with_capacity(5);
        for column in [
            &update.credentials,
            &update.env_vars,
            &update.static_headers,
            &update.env,
        ] {
            data.push(column.as_ref().map(serde_json::to_string).transpose()?);
        }
        data.push(Some(update.updated_by.clone()));
        let table = self.servers.table(|access| McpUpdateActions {
            access,
            update_sql: SERVER_UPDATE_SQL,
        })?;
        table.update(data, vec![server_id.to_owned()]).await?;
        Ok(())
    }

    async fn oauth_client_rows(&self) -> Result<Vec<StoredOAuthClientRow>, Self::Error> {
        let client = self.oauth_clients.access()?.read.session().await?;
        let rows = client
            .query(OAUTH_ROWS_SQL, &[])
            .await
            .map_err(backend::<Infallible>)?;
        let found = rows
            .iter()
            .map(|row| {
                Ok(StoredOAuthClientRow {
                    server_id: row.try_get("server_id")?,
                    credentials: row.try_get("credentials")?,
                })
            })
            .collect::<Result<Vec<_>, DriverError>>()
            .map_err(backend::<Infallible>)?;
        Ok(found)
    }

    async fn update_oauth_client(
        &self,
        server_id: &str,
        credentials: &JsonValue,
    ) -> Result<(), Self::Error> {
        let data = vec![Some(serde_json::to_string(credentials)?)];
        let table = self.oauth_clients.table(|access| McpUpdateActions {
            access,
            update_sql: OAUTH_UPDATE_SQL,
        })?;
        table.update(data, vec![server_id.to_owned()]).await?;
        Ok(())
    }

    async fn user_credential_rows(&self) -> Result<Vec<StoredUserCredentialRow>, Self::Error> {
        let client = self.user_credentials.access()?.read.session().await?;
        let rows = client
            .query(USER_CREDENTIAL_ROWS_SQL, &[])
            .await
            .map_err(backend::<Infallible>)?;
        let found = rows
            .iter()
            .map(|row| {
                Ok(StoredUserCredentialRow {
                    user_id: row.try_get("user_id")?,
                    server_id: row.try_get("server_id")?,
                    credential_b64: row.try_get("credential_b64")?,
                })
            })
            .collect::<Result<Vec<_>, DriverError>>()
            .map_err(backend::<Infallible>)?;
        Ok(found)
    }

    async fn update_user_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> Result<(), Self::Error> {
        let table = self.user_credentials.table(|access| McpUpdateActions {
            access,
            update_sql: USER_CREDENTIAL_UPDATE_SQL,
        })?;
        let keys = vec![user_id.to_owned(), server_id.to_owned()];
        table
            .update(vec![Some(credential_b64.to_owned())], keys)
            .await?;
        Ok(())
    }

    async fn user_env_var_rows(&self) -> Result<Vec<StoredUserEnvVarRow>, Self::Error> {
        let client = self.user_env_vars.access()?.read.session().await?;
        let rows = client
            .query(USER_ENV_VAR_ROWS_SQL, &[])
            .await
            .map_err(backend::<Infallible>)?;
        let found = rows
            .iter()
            .map(|row| {
                Ok(StoredUserEnvVarRow {
                    user_id: row.try_get("user_id")?,
                    server_id: row.try_get("server_id")?,
                    values_b64: row.try_get("values_b64")?,
                })
            })
            .collect::<Result<Vec<_>, DriverError>>()
            .map_err(backend::<Infallible>)?;
        Ok(found)
    }

    async fn update_user_env_var(
        &self,
        user_id: &str,
        server_id: &str,
        values_b64: &str,
    ) -> Result<(), Self::Error> {
        let table = self.user_env_vars.table(|access| McpUpdateActions {
            access,
            update_sql: USER_ENV_VAR_UPDATE_SQL,
        })?;
        let keys = vec![user_id.to_owned(), server_id.to_owned()];
        table
            .update(vec![Some(values_b64.to_owned())], keys)
            .await?;
        Ok(())
    }
}
