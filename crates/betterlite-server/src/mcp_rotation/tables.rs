use std::convert::Infallible;

use betterlite_db::auxiliary_repository::bindings::{
    MCPServerOAuthClient, MCPServerTable, MCPUserCredentials,
};
use betterlite_db::auxiliary_repository::repository::{
    PrismaTableRepository, TableAccess, TableBinding, TableError,
};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::decode::decode_mcp_stored_contract;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_redis::config_sync::table_actions::TableActions;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio_postgres::types::ToSql;

use super::error::RotationCodecError;
use super::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};

// Each table is read whole, as find_many with no where does; the row arrives as one JSON object.
const SERVER_ROWS_SQL: &str = r#"SELECT to_jsonb(t)::text FROM (SELECT server_id, credentials, env_vars, static_headers, env FROM "LiteLLM_MCPServerTable") t"#;
const OAUTH_ROWS_SQL: &str = r#"SELECT to_jsonb(t)::text FROM (SELECT server_id, credentials FROM "LiteLLM_MCPServerOAuthClient") t"#;
const USER_CREDENTIAL_ROWS_SQL: &str = r#"SELECT to_jsonb(t)::text FROM (SELECT user_id, server_id, credential_b64 FROM "LiteLLM_MCPUserCredentials") t"#;
const USER_ENV_VAR_ROWS_SQL: &str = r#"SELECT to_jsonb(t)::text FROM (SELECT user_id, server_id, values_b64 FROM "LiteLLM_MCPUserEnvVars") t"#;

// A NULL parameter keeps the stored column; updated_at moves as Prisma moves it on every update.
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
    #[error(transparent)]
    Codec(#[from] RotationCodecError),
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
        let params: Vec<&(dyn ToSql + Sync)> = keys
            .iter()
            .map(|key| key as &(dyn ToSql + Sync))
            .chain(data.iter().map(|value| value as &(dyn ToSql + Sync)))
            .collect();
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

async fn read_rows<T, B>(
    repository: &PrismaTableRepository<'_, B>,
    sql: &'static str,
) -> Result<Vec<T>, PostgresRotationError>
where
    T: DeserializeOwned + Send,
    B: TableBinding + Sync,
{
    let client = repository.access()?.read.session().await?;
    let rows = client
        .query(sql, &[])
        .await
        .map_err(backend::<Infallible>)?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: &str = row.try_get(0).map_err(backend::<Infallible>)?;
        decode_mcp_stored_contract(text.as_bytes()).map_err(RotationCodecError::Blob)?;
        found.push(serde_json::from_str(text).map_err(PostgresRotationError::Json)?);
    }
    Ok(found)
}

macro_rules! user_update {
    ($method:ident, $repository:ident, $sql:ident, $value:ident) => {
        async fn $method(
            &self,
            user_id: &str,
            server_id: &str,
            $value: &str,
        ) -> Result<(), Self::Error> {
            let table = self.$repository.table(|access| McpUpdateActions {
                access,
                update_sql: $sql,
            })?;
            let keys = vec![user_id.to_owned(), server_id.to_owned()];
            table
                .update_many(vec![Some($value.to_owned())], keys)
                .await?;
            Ok(())
        }
    };
}

pub struct PostgresRotationStore<'db> {
    pub servers: PrismaTableRepository<'db, MCPServerTable>,
    pub oauth_clients: PrismaTableRepository<'db, MCPServerOAuthClient>,
    pub user_credentials: PrismaTableRepository<'db, MCPUserCredentials>,
    pub user_env_vars: PrismaTableRepository<'db, McpUserEnvVars>,
}

impl McpRotationStore for PostgresRotationStore<'_> {
    type Error = PostgresRotationError;

    async fn server_rows(&self) -> Result<Vec<StoredServerRow>, Self::Error> {
        read_rows(&self.servers, SERVER_ROWS_SQL).await
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
        table.update_many(data, vec![server_id.to_owned()]).await?;
        Ok(())
    }

    async fn oauth_client_rows(&self) -> Result<Vec<StoredOAuthClientRow>, Self::Error> {
        read_rows(&self.oauth_clients, OAUTH_ROWS_SQL).await
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
        table.update_many(data, vec![server_id.to_owned()]).await?;
        Ok(())
    }

    async fn user_credential_rows(&self) -> Result<Vec<StoredUserCredentialRow>, Self::Error> {
        read_rows(&self.user_credentials, USER_CREDENTIAL_ROWS_SQL).await
    }

    user_update!(
        update_user_credential,
        user_credentials,
        USER_CREDENTIAL_UPDATE_SQL,
        credential_b64
    );

    async fn user_env_var_rows(&self) -> Result<Vec<StoredUserEnvVarRow>, Self::Error> {
        read_rows(&self.user_env_vars, USER_ENV_VAR_ROWS_SQL).await
    }

    user_update!(
        update_user_env_var,
        user_env_vars,
        USER_ENV_VAR_UPDATE_SQL,
        values_b64
    );
}
