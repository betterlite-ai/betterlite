use betterlite_db::auxiliary_repository::bindings::AccessGroupTable;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use betterlite_db::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use betterlite_db::postgres_store::errors::backend;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::{
    DeploymentCountRow, ModelGrantDatabase, ModelNameRewrite, ModelSyncError, TouchedGroupRow,
};

pub const BACKING_DEPLOYMENTS_SQL: &str = "SELECT COUNT(*)::int AS deployment_count FROM \"LiteLLM_ProxyModelTable\" WHERE \"model_name\" = $1";
pub const REPLACE_MODEL_NAME_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"access_model_names\" = array_replace(array_remove(\"access_model_names\", $2), $1, $2) WHERE $1 = ANY(\"access_model_names\") RETURNING \"access_group_id\"";
pub const APPEND_MODEL_NAME_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"access_model_names\" = array_append(\"access_model_names\", $2) WHERE $1 = ANY(\"access_model_names\") AND NOT ($2 = ANY(\"access_model_names\")) RETURNING \"access_group_id\"";
pub const REMOVE_MODEL_NAME_SQL: &str = "UPDATE \"LiteLLM_AccessGroupTable\" SET \"access_model_names\" = array_remove(\"access_model_names\", $1) WHERE $1 = ANY(\"access_model_names\") RETURNING \"access_group_id\"";

impl ModelNameRewrite {
    const fn sql(&self) -> &'static str {
        match self {
            Self::Append { .. } => APPEND_MODEL_NAME_SQL,
            Self::Replace { .. } => REPLACE_MODEL_NAME_SQL,
            Self::Remove { .. } => REMOVE_MODEL_NAME_SQL,
        }
    }

    fn parameters(&self) -> Vec<&(dyn ToSql + Sync)> {
        match self {
            Self::Append { old_name, new_name } | Self::Replace { old_name, new_name } => {
                let parameters: [&(dyn ToSql + Sync); 2] = [old_name, new_name];
                parameters.to_vec()
            }
            Self::Remove { model_name } => {
                let parameters: [&(dyn ToSql + Sync); 1] = [model_name];
                parameters.to_vec()
            }
        }
    }
}

pub struct PostgresModelGrants<'db> {
    pub client: Option<PrismaClient<'db>>,
}

impl ModelGrantDatabase for PostgresModelGrants<'_> {
    async fn backing_deployments(
        &self,
        model_name: &str,
    ) -> Result<Vec<DeploymentCountRow>, ModelSyncError> {
        let parameters: [&(dyn ToSql + Sync); 1] = [&model_name];
        let rows = writer_rows(self.client, BACKING_DEPLOYMENTS_SQL, &parameters).await?;
        rows.iter().map(DeploymentCountRow::from_row).collect()
    }

    async fn rewrite_model_names(
        &self,
        rewrite: &ModelNameRewrite,
    ) -> Result<Vec<TouchedGroupRow>, ModelSyncError> {
        let parameters = rewrite.parameters();
        let rows = writer_rows(self.client, rewrite.sql(), &parameters).await?;
        rows.iter().map(TouchedGroupRow::from_row).collect()
    }
}

impl DeploymentCountRow {
    fn from_row(row: &Row) -> Result<Self, ModelSyncError> {
        let deployment_count = row
            .try_get(0)
            .map_err(|source| ModelSyncError::Database(backend(source)))?;
        Ok(Self { deployment_count })
    }
}

impl TouchedGroupRow {
    fn from_row(row: &Row) -> Result<Self, ModelSyncError> {
        let access_group_id = row
            .try_get(0)
            .map_err(|source| ModelSyncError::Database(backend(source)))?;
        Ok(Self { access_group_id })
    }
}

async fn writer_rows(
    client: Option<PrismaClient<'_>>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, ModelSyncError> {
    let repository = PrismaTableRepository::new(
        client,
        redis_config_change_publisher(None),
        AccessGroupTable,
    );
    let resolved = repository.prisma_client()?;
    // B714 pins raw SQL writes to the writer handle even while the writer reports degraded
    let pinned = WriterPinnedClient::Writer(resolved.writer);
    let store = writer_wrapper(pinned);
    let session = store.session().await?;
    session
        .query(sql, parameters)
        .await
        .map_err(|source| ModelSyncError::Database(backend(source)))
}
