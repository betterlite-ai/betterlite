use deadpool_postgres::GenericClient;
use tokio_postgres::Row;

use crate::postgres_store::errors::{DbError, backend};

use super::errors::ToolRecordsError;
use super::records::{ToolInputPolicy, ToolPolicyOverrideRow};

// LiteLLM appends each permission's keys and then its teams; the sort makes that projection stable
const OVERRIDE_SQL: &str = "SELECT perm.\"object_permission_id\" AS override_id, false AS is_team, tok.\"token\" AS key_hash, tok.\"key_alias\" AS key_alias, NULL::text AS team_id FROM \"LiteLLM_ObjectPermissionTable\" AS perm JOIN \"LiteLLM_VerificationToken\" AS tok ON tok.\"object_permission_id\" = perm.\"object_permission_id\" WHERE $1 = ANY(perm.\"blocked_tools\") UNION ALL SELECT perm.\"object_permission_id\", true, NULL::text, team.\"team_alias\", team.\"team_id\" FROM \"LiteLLM_ObjectPermissionTable\" AS perm JOIN \"LiteLLM_TeamTable\" AS team ON team.\"object_permission_id\" = perm.\"object_permission_id\" WHERE $1 = ANY(perm.\"blocked_tools\") ORDER BY 1, 2, 3 NULLS LAST, 4 NULLS LAST, 5 NULLS LAST";

pub async fn list_overrides_for_tool<Client>(
    client: Option<&Client>,
    tool_name: &str,
) -> Vec<ToolPolicyOverrideRow>
where
    Client: GenericClient + Sync,
{
    match fetch_overrides(client, tool_name).await {
        Ok(overrides) => overrides,
        Err(error) => {
            // LiteLLM answers a failed override read with an empty list rather than an error
            tracing::error!("tool_registry_writer list_overrides_for_tool error: {error}");
            Vec::new()
        }
    }
}

async fn fetch_overrides<Client>(
    client: Option<&Client>,
    tool_name: &str,
) -> Result<Vec<ToolPolicyOverrideRow>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    let connected = client.ok_or_else(|| ToolRecordsError::Overrides {
        source: DbError::NoDbConnected,
    })?;
    let rows = connected
        .query(OVERRIDE_SQL, &[&tool_name])
        .await
        .map_err(override_failure)?;
    rows.iter()
        .map(|row| override_row(row, tool_name))
        .collect()
}

fn override_row(row: &Row, tool_name: &str) -> Result<ToolPolicyOverrideRow, ToolRecordsError> {
    let override_id: String = row.try_get(0).map_err(override_failure)?;
    let key_hash: Option<String> = row.try_get(2).map_err(override_failure)?;
    let key_alias: Option<String> = row.try_get(3).map_err(override_failure)?;
    let team_id: Option<String> = row.try_get(4).map_err(override_failure)?;
    Ok(ToolPolicyOverrideRow {
        override_id,
        tool_name: tool_name.to_owned(),
        team_id,
        key_hash,
        input_policy: ToolInputPolicy::Blocked,
        key_alias,
        created_at: None,
        updated_at: None,
    })
}

fn override_failure(error: tokio_postgres::Error) -> ToolRecordsError {
    ToolRecordsError::Overrides {
        source: backend(error),
    }
}
