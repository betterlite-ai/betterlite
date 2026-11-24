pub mod access_groups;
pub mod memberships;

mod tests;

use std::slice::from_ref;

use betterlite_db::postgres_store::errors::backend;
use deadpool_postgres::GenericClient;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use self::memberships::Membership;
use super::error::AccessGroupStoreError;
use super::records::AccessGroupRecord;
use super::tables::{AccessGroupTable, AccessGroupTx, AgentTable, KeyTable, TeamTable};

// The columns every access group read and write returns, so a record always carries the row Prisma hands back
pub(super) const ACCESS_GROUP_COLUMNS: &str = concat!(
    r#""access_group_id", "access_group_name", "description", "access_model_names", "#,
    r#""access_mcp_server_ids", "access_agent_ids", "assigned_team_ids", "assigned_key_ids", "#,
    r#""created_at", "created_by", "updated_at", "updated_by""#,
);

pub struct PgAccessGroupTables<'db, C> {
    pub client: &'db C,
}

impl<C: GenericClient + Sync> PgAccessGroupTables<'_, C> {
    async fn query(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, AccessGroupStoreError> {
        self.client
            .query(statement, parameters)
            .await
            .map_err(|source| AccessGroupStoreError::Database(backend(source)))
    }

    async fn execute_changing_one(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<(), AccessGroupStoreError> {
        let affected = self
            .client
            .execute(statement, parameters)
            .await
            .map_err(|source| AccessGroupStoreError::Database(backend(source)))?;
        if affected == 0 {
            return Err(AccessGroupStoreError::RecordNotFound);
        }
        Ok(())
    }

    async fn optional_record(
        &self,
        condition: &str,
        value: &(dyn ToSql + Sync),
    ) -> Result<Option<AccessGroupRecord>, AccessGroupStoreError> {
        let statement = format!(
            r#"SELECT {ACCESS_GROUP_COLUMNS} FROM "LiteLLM_AccessGroupTable" WHERE {condition}"#
        );
        let rows = self.query(&statement, &[value]).await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        AccessGroupRecord::from_row(row).map(Some)
    }

    async fn write_list(
        &self,
        table: Membership,
        key: &str,
        ids: &[String],
    ) -> Result<(), AccessGroupStoreError> {
        let statement = table.statement();
        let parameters: [&(dyn ToSql + Sync); 2] = [&key, &ids];
        self.execute_changing_one(statement, &parameters).await
    }

    async fn carrying_records<R>(
        &self,
        statement: &str,
        access_group_id: &str,
        decode: fn(&Row) -> Result<R, AccessGroupStoreError>,
    ) -> Result<Vec<R>, AccessGroupStoreError> {
        // Prisma's hasSome sends the ids as a list, and the query's parameter is text[]
        let groups: &[&str] = from_ref(&access_group_id);
        let parameters: [&(dyn ToSql + Sync); 1] = [&groups];
        let rows = self.query(statement, &parameters).await?;
        rows.iter().map(decode).collect()
    }
}

impl<C: GenericClient + Sync> AccessGroupTx for PgAccessGroupTables<'_, C> {
    fn litellm_accessgrouptable(&self) -> impl AccessGroupTable {
        PgAccessGroupTables {
            client: self.client,
        }
    }

    fn litellm_teamtable(&self) -> impl TeamTable {
        PgAccessGroupTables {
            client: self.client,
        }
    }

    fn litellm_verificationtoken(&self) -> impl KeyTable {
        PgAccessGroupTables {
            client: self.client,
        }
    }

    fn litellm_agentstable(&self) -> impl AgentTable {
        PgAccessGroupTables {
            client: self.client,
        }
    }
}
