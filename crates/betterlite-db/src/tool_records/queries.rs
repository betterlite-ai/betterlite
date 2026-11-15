use std::collections::{BTreeMap, BTreeSet};

use deadpool_postgres::GenericClient;
use tokio_postgres::Row;

use crate::key_repository::decode::decode_token_record;
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
const TOKEN_OWNER_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_VerificationToken\" AS entry WHERE entry.\"token\" = ANY($1::text[])";

pub async fn list_tools<Client>(
    client: Option<&Client>,
    input_policy: Option<ToolInputPolicy>,
) -> Vec<LiteLLMToolTableRow>
where
    Client: GenericClient + Sync,
{
    match fetch_tools(client, input_policy).await {
        Ok(tools) => tools,
        Err(error) => {
            // LiteLLM answers a failed read with an empty list rather than an error
            tracing::error!("tool_registry_writer list_tools error: {error}");
            Vec::new()
        }
    }
}

async fn fetch_tools<Client>(
    client: Option<&Client>,
    input_policy: Option<ToolInputPolicy>,
) -> Result<Vec<LiteLLMToolTableRow>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    let mut tools = tool_rows(client, input_policy).await?;
    let key_hashes: BTreeSet<String> = tools
        .iter()
        .filter_map(|tool| tool.key_hash.clone())
        .filter(|key| !key.is_empty())
        .collect();
    let owners = key_owners_or_none(client, &key_hashes).await?;
    attach_owners(&mut tools, &owners);
    Ok(tools)
}

async fn tool_rows<Client>(
    client: Option<&Client>,
    input_policy: Option<ToolInputPolicy>,
) -> Result<Vec<LiteLLMToolTableRow>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    let client = client.ok_or_else(|| ToolRecordsError::Tools {
        source: DbError::NoDbConnected,
    })?;
    let policy = input_policy.map(ToolInputPolicy::as_str);
    let rows = client
        .query(LIST_TOOLS_SQL, &[&policy])
        .await
        .map_err(|source| ToolRecordsError::Tools {
            source: backend(source),
        })?;
    rows.iter().map(decode_tool_row).collect()
}

fn decode_tool_row(row: &Row) -> Result<LiteLLMToolTableRow, ToolRecordsError> {
    let text: String = row.try_get(0).map_err(|source| ToolRecordsError::Tools {
        source: backend(source),
    })?;
    decode_tool_record(text.as_bytes()).map_err(|source| ToolRecordsError::Tools {
        source: DbError::InvalidRecord { source },
    })
}

pub async fn get_tool<Client>(
    client: Option<&Client>,
    tool_name: &str,
) -> Option<LiteLLMToolTableRow>
where
    Client: GenericClient + Sync,
{
    match fetch_tool(client, tool_name).await {
        Ok(tool) => tool,
        Err(error) => {
            tracing::error!("tool_registry_writer get_tool error: {error}");
            None
        }
    }
}

async fn fetch_tool<Client>(
    client: Option<&Client>,
    tool_name: &str,
) -> Result<Option<LiteLLMToolTableRow>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    let client = client.ok_or_else(|| ToolRecordsError::Tools {
        source: DbError::NoDbConnected,
    })?;
    let row = client
        .query_opt(GET_TOOL_SQL, &[&tool_name])
        .await
        .map_err(|source| ToolRecordsError::Tools {
            source: backend(source),
        })?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut tool = decode_tool_row(&row)?;
    let key_hashes: BTreeSet<String> = tool
        .key_hash
        .iter()
        .filter(|key| !key.is_empty())
        .cloned()
        .collect();
    let owners = key_owners_or_none(Some(client), &key_hashes).await?;
    attach_owners(std::slice::from_mut(&mut tool), &owners);
    Ok(Some(tool))
}

async fn key_owners<Client>(
    client: Option<&Client>,
    key_hashes: &BTreeSet<String>,
) -> Result<BTreeMap<String, ToolDiscoveryUser>, ToolRecordsError>
where
    Client: GenericClient + Sync,
{
    if key_hashes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let client = client.ok_or_else(|| ToolRecordsError::Tokens {
        source: DbError::NoDbConnected,
    })?;
    let hashes: Vec<String> = key_hashes.iter().cloned().collect();
    let rows = client
        .query(TOKEN_OWNER_SQL, &[&hashes])
        .await
        .map_err(|source| ToolRecordsError::Tokens {
            source: backend(source),
        })?;
    let mut tokens = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row.try_get(0).map_err(|source| ToolRecordsError::Tokens {
            source: backend(source),
        })?;
        tokens.push(decode_token_record(text.as_bytes()).map_err(|source| {
            ToolRecordsError::Tokens {
                source: DbError::InvalidRecord { source },
            }
        })?);
    }
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
    client: Option<&Client>,
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
