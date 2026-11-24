use betterlite_db::postgres_store::errors::DbError;
use deadpool_postgres::GenericClient;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::access_group_read::protocols::error::AccessGroupStoreError;
use crate::access_group_read::protocols::records::{
    AccessGroupChanges, AccessGroupRecord, FieldWrite, NewAccessGroup,
};
use crate::access_group_read::protocols::tables::AccessGroupTable;

use super::{ACCESS_GROUP_COLUMNS, PgAccessGroupTables};

const DELETE_ACCESS_GROUP_SQL: &str =
    r#"DELETE FROM "LiteLLM_AccessGroupTable" WHERE "access_group_id" = $1"#;

impl<C: GenericClient + Sync> AccessGroupTable for PgAccessGroupTables<'_, C> {
    async fn find_by_id(
        &self,
        access_group_id: &str,
    ) -> Result<Option<AccessGroupRecord>, AccessGroupStoreError> {
        self.optional_record(r#""access_group_id" = $1"#, &access_group_id)
            .await
    }

    async fn find_by_name(
        &self,
        access_group_name: &str,
    ) -> Result<Option<AccessGroupRecord>, AccessGroupStoreError> {
        self.optional_record(r#""access_group_name" = $1"#, &access_group_name)
            .await
    }

    async fn list_newest_first(&self) -> Result<Vec<AccessGroupRecord>, AccessGroupStoreError> {
        let statement = format!(
            r#"SELECT {ACCESS_GROUP_COLUMNS} FROM "LiteLLM_AccessGroupTable" ORDER BY "created_at" DESC"#
        );
        let rows = self.query(&statement, &[]).await?;
        rows.iter().map(AccessGroupRecord::from_row).collect()
    }

    async fn create(
        &self,
        new: &NewAccessGroup,
    ) -> Result<AccessGroupRecord, AccessGroupStoreError> {
        // @default(uuid()) runs in Prisma's client and the column has no default, so the insert supplies the id
        let access_group_id = Uuid::new_v4().to_string();
        // created_at and updated_at keep their CURRENT_TIMESTAMP defaults, the instant @default(now()) writes
        let statement = format!(
            r#"INSERT INTO "LiteLLM_AccessGroupTable" ("access_group_id", "access_group_name", "description", "access_model_names", "access_mcp_server_ids", "access_agent_ids", "assigned_team_ids", "assigned_key_ids", "created_by", "updated_by") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {ACCESS_GROUP_COLUMNS}"#
        );
        let parameters: [&(dyn ToSql + Sync); 10] = [
            &access_group_id,
            &new.access_group_name,
            &new.description,
            &new.access_model_names,
            &new.access_mcp_server_ids,
            &new.access_agent_ids,
            &new.assigned_team_ids,
            &new.assigned_key_ids,
            &new.created_by,
            &new.updated_by,
        ];
        let rows = self.query(&statement, &parameters).await?;
        let Some(row) = rows.first() else {
            return Err(AccessGroupStoreError::Database(DbError::CreateNull));
        };
        AccessGroupRecord::from_row(row)
    }

    async fn update(
        &self,
        access_group_id: &str,
        changes: &AccessGroupChanges,
    ) -> Result<AccessGroupRecord, AccessGroupStoreError> {
        let (statement, parameters) = update_statement(&access_group_id, changes);
        let rows = self.query(&statement, &parameters).await?;
        let Some(row) = rows.first() else {
            return Err(AccessGroupStoreError::RecordNotFound);
        };
        AccessGroupRecord::from_row(row)
    }

    async fn delete(&self, access_group_id: &str) -> Result<(), AccessGroupStoreError> {
        let parameters: [&(dyn ToSql + Sync); 1] = [&access_group_id];
        self.execute_changing_one(DELETE_ACCESS_GROUP_SQL, &parameters)
            .await
    }
}

impl AccessGroupRecord {
    pub(super) fn from_row(row: &Row) -> Result<Self, AccessGroupStoreError> {
        Ok(Self {
            access_group_id: row.try_get("access_group_id")?,
            access_group_name: row.try_get("access_group_name")?,
            description: row.try_get("description")?,
            access_model_names: row.try_get("access_model_names")?,
            access_mcp_server_ids: row.try_get("access_mcp_server_ids")?,
            access_agent_ids: row.try_get("access_agent_ids")?,
            assigned_team_ids: row.try_get("assigned_team_ids")?,
            assigned_key_ids: row.try_get("assigned_key_ids")?,
            created_at: row.try_get("created_at")?,
            created_by: row.try_get("created_by")?,
            updated_at: row.try_get("updated_at")?,
            updated_by: row.try_get("updated_by")?,
        })
    }
}

// Prisma writes updated_by and the columns its data map names; updated_at always moves, and nothing else is touched
fn update_statement<'change>(
    access_group_id: &'change &'change str,
    changes: &'change AccessGroupChanges,
) -> (String, Vec<&'change (dyn ToSql + Sync)>) {
    let mut assignments = vec![
        String::from(r#""updated_by" = $2"#),
        String::from(r#""updated_at" = (NOW() AT TIME ZONE 'UTC')"#),
    ];
    // the id arrives by reference to the caller's binding, so the returned parameters can borrow it
    let mut parameters: Vec<&'change (dyn ToSql + Sync)> =
        vec![access_group_id, &changes.updated_by];
    if let Some(name) = &changes.access_group_name {
        push_parameter(&mut assignments, &mut parameters, "access_group_name", name);
    }
    match &changes.description {
        FieldWrite::Unchanged => {}
        FieldWrite::Null => assignments.push(String::from(r#""description" = NULL"#)),
        FieldWrite::Set(text) => {
            push_parameter(&mut assignments, &mut parameters, "description", text);
        }
    }
    push_lists(&mut assignments, &mut parameters, changes);
    let set = assignments.join(", ");
    let statement = format!(
        r#"UPDATE "LiteLLM_AccessGroupTable" SET {set} WHERE "access_group_id" = $1 RETURNING {ACCESS_GROUP_COLUMNS}"#
    );
    (statement, parameters)
}

fn push_parameter<'change>(
    assignments: &mut Vec<String>,
    parameters: &mut Vec<&'change (dyn ToSql + Sync)>,
    column: &str,
    value: &'change (dyn ToSql + Sync),
) {
    let position = parameters.len().saturating_add(1);
    assignments.push(format!(r#""{column}" = ${position}"#));
    parameters.push(value);
}

fn push_lists<'change>(
    assignments: &mut Vec<String>,
    parameters: &mut Vec<&'change (dyn ToSql + Sync)>,
    changes: &'change AccessGroupChanges,
) {
    let lists = [
        ("access_model_names", &changes.access_model_names),
        ("access_mcp_server_ids", &changes.access_mcp_server_ids),
        ("access_agent_ids", &changes.access_agent_ids),
        ("assigned_team_ids", &changes.assigned_team_ids),
        ("assigned_key_ids", &changes.assigned_key_ids),
    ];
    for (column, list) in lists {
        if let Some(values) = list {
            push_parameter(assignments, parameters, column, values);
        }
    }
}

mod tests;
