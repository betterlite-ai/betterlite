use std::collections::{BTreeMap, BTreeSet};

use deadpool_postgres::GenericClient;
use tokio_postgres::Row;

use crate::key_repository::table::converted::ConvertedTokenTable;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::KeyColumn;
use crate::user_repository::tables::USER_TABLE;

use super::decode::decode_tool_record;
use super::errors::ToolRecordsError;
use super::owners::{attach_owners, owners_from_rows};
use super::records::{LiteLLMToolTableRow, ToolDiscoveryUser, ToolInputPolicy};

// LiteLLM leaves rows that share a created_at unordered; the id keeps the read deterministic
const LIST_TOOLS_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ToolTable\" AS entry WHERE $1::text IS NULL OR entry.\"input_policy\" = $1 ORDER BY entry.\"created_at\" DESC, entry.\"tool_id\"";
const GET_TOOL_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ToolTable\" AS entry WHERE entry.\"tool_name\" = $1";

pub async fn list_tools<Client>(
    client: Option<&Client>,
    input_policy: Option<ToolInputPolicy>,
) -> Vec<LiteLLMToolTableRow>
where
    Client: GenericClient + Sync,
{
    let policy = input_policy.map(ToolInputPolicy::as_str);
    match read_tools(client, LIST_TOOLS_SQL, policy).await {
        Ok(tools) => tools,
        Err(error) => {
            // LiteLLM answers a failed read with an empty list rather than an error
            tracing::error!("tool_registry_writer list_tools error: {error}");
            Vec::new()
        }
    }
}

pub async fn get_tool<Client>(
    client: Option<&Client>,
    tool_name: &str,
) -> Option<LiteLLMToolTableRow>
where
    Client: GenericClient + Sync,
{
    read_tools(client, GET_TOOL_SQL, Some(tool_name))
        .await
        .inspect_err(|error| tracing::error!("tool_registry_writer get_tool error: {error}"))
        .ok()
        .and_then(|tools| tools.into_iter().next())
}

async fn read_tools<Client>(
    client: Option<&Client>,
    sql: &str,
    parameter: Option<&str>,
) -> Result<Vec<LiteLLMToolTableRow>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    let connected = client.ok_or_else(|| ToolRecordsError::Tools {
        source: DbError::NoDbConnected,
    })?;
    let rows = connected
        .query(sql, &[&parameter])
        .await
        .map_err(|source| ToolRecordsError::Tools {
            source: backend(source),
        })?;
    let mut tools = rows
        .iter()
        .map(decode_tool_row)
        .collect::<Result<Vec<_>, _>>()?;
    let key_hashes: BTreeSet<String> = tools
        .iter()
        .filter_map(|tool| tool.key_hash.clone())
        .filter(|key| !key.is_empty())
        .collect();
    let owners = key_owners_or_none(connected, &key_hashes).await?;
    attach_owners(&mut tools, &owners);
    Ok(tools)
}

fn decode_tool_row(row: &Row) -> Result<LiteLLMToolTableRow, ToolRecordsError> {
    let text: String = row.try_get(0).map_err(|source| ToolRecordsError::Tools {
        source: backend(source),
    })?;
    decode_tool_record(text.as_bytes()).map_err(|source| ToolRecordsError::Tools {
        source: DbError::InvalidRecord { source },
    })
}

async fn key_owners<Client>(
    client: &Client,
    key_hashes: &BTreeSet<String>,
) -> Result<BTreeMap<String, ToolDiscoveryUser>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    if key_hashes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let hashes: Vec<String> = key_hashes.iter().cloned().collect();
    let tokens = Repository {
        client: Some(client),
        table: ConvertedTokenTable,
    }
    .find_many_in(KeyColumn::Id, hashes)
    .await
    .map_err(|source| ToolRecordsError::Tokens { source })?;
    let owner_ids: Vec<String> = tokens
        .iter()
        .filter_map(|token| token.user_id.clone())
        .filter(|user_id| !user_id.is_empty())
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    let users = Repository {
        client: Some(client),
        table: USER_TABLE,
    }
    .find_many_in(KeyColumn::Id, owner_ids)
    .await
    .map_err(|source| ToolRecordsError::Users { source })?;
    Ok(owners_from_rows(&tokens, &users))
}

async fn key_owners_or_none<Client>(
    client: &Client,
    key_hashes: &BTreeSet<String>,
) -> Result<BTreeMap<String, ToolDiscoveryUser>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    match key_owners(client, key_hashes).await {
        Ok(owners) => Ok(owners),
        Err(error) if error.is_database_failure() => {
            tracing::error!("tool_registry_writer owner lookup error: {error}");
            Ok(BTreeMap::new())
        }
        Err(error) => Err(error),
    }
}
