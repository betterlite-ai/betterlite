use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::{Mutex, PoisonError};

use crate::database_faults::native::is_native_transport_error;
use crate::database_faults::reconnect::{DbReconnect, RetryPolicy, call_with_db_reconnect_retry};
use crate::database_faults::{DbError, ErrorType};
use crate::postgres_store::errors::{DbError as NativeDbError, backend};
use crate::postgres_store::session::PostgresStore;
use crate::tool_records::decode::decode_tool_record;

use super::errors::ToolPolicyStoreError;
use super::records::BlockedToolRow;
use super::registry::{ToolPolicyMaps, ToolPolicyRegistry, blocked_tools_map, tool_policy_maps};

const TOOLS_SQL: &str =
    "SELECT row_to_json(entry)::text AS row_text FROM \"LiteLLM_ToolTable\" AS entry";
const PERMISSIONS_SQL: &str =
    "SELECT \"object_permission_id\", \"blocked_tools\" FROM \"LiteLLM_ObjectPermissionTable\"";
const TOOLS_REASON: &str = "sync_tool_policy_from_db_tools_lookup_failure";
const PERMISSIONS_REASON: &str = "sync_tool_policy_from_db_perms_lookup_failure";

pub struct ToolPolicyStore<'store> {
    pub database: &'store PostgresStore,
    pub registry: &'store Mutex<ToolPolicyRegistry>,
}

pub async fn sync_tool_policy_from_db<Client>(
    store: &ToolPolicyStore<'_>,
    reconnect: Option<&mut Client>,
) -> Result<(), ToolPolicyStoreError>
where
    Client: DbReconnect + Send,
{
    match sync_tool_policy(store, reconnect).await {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::error!("ToolPolicyRegistry sync_tool_policy_from_db error: {error}");
            Err(error)
        }
    }
}

async fn sync_tool_policy<Client>(
    store: &ToolPolicyStore<'_>,
    mut reconnect: Option<&mut Client>,
) -> Result<(), ToolPolicyStoreError>
where
    Client: DbReconnect + Send,
{
    let maps = load_policy_maps(store.database, reconnect.as_deref_mut()).await?;
    let tool_count = maps.input_policies.len();
    {
        let mut registry = store
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registry.input_policies = maps.input_policies;
        registry.output_policies = maps.output_policies;
    }
    let blocked_tools = load_blocked_tools(store.database, reconnect.as_deref_mut()).await?;
    let permission_count = blocked_tools.len();
    let mut registry = store
        .registry
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    registry.blocked_tools_by_permission = blocked_tools;
    registry.initialized = true;
    tracing::info!(
        "ToolPolicyRegistry: synced {tool_count} tool policies and {permission_count} object permissions from DB"
    );
    Ok(())
}

async fn load_policy_maps<Client>(
    database: &PostgresStore,
    reconnect: Option<&mut Client>,
) -> Result<ToolPolicyMaps, ToolPolicyStoreError>
where
    Client: DbReconnect + Send,
{
    let texts = call_with_db_reconnect_retry(
        reconnect,
        || read_tool_rows(database),
        RetryPolicy {
            reason: TOOLS_REASON,
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await
    .map_err(|source| ToolPolicyStoreError::Read { source })?;
    let mut tools = Vec::with_capacity(texts.len());
    for text in &texts {
        tools.push(
            decode_tool_record(text.as_bytes())
                .map_err(|source| ToolPolicyStoreError::Record { source })?,
        );
    }
    Ok(tool_policy_maps(&tools))
}

async fn load_blocked_tools<Client>(
    database: &PostgresStore,
    reconnect: Option<&mut Client>,
) -> Result<BTreeMap<String, Vec<String>>, ToolPolicyStoreError>
where
    Client: DbReconnect + Send,
{
    let rows = call_with_db_reconnect_retry(
        reconnect,
        || read_blocked_rows(database),
        RetryPolicy {
            reason: PERMISSIONS_REASON,
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await
    .map_err(|source| ToolPolicyStoreError::Read { source })?;
    Ok(blocked_tools_map(&rows))
}

async fn read_tool_rows(database: &PostgresStore) -> Result<Vec<String>, DbError> {
    let client = database
        .session()
        .await
        .map_err(|source| read_fault(&source))?;
    let rows = client
        .query(TOOLS_SQL, &[])
        .await
        .map_err(|source| read_fault(&backend(source)))?;
    let mut texts = Vec::with_capacity(rows.len());
    for row in &rows {
        texts.push(
            row.try_get::<_, String>("row_text")
                .map_err(|source| read_fault(&backend(source)))?,
        );
    }
    Ok(texts)
}

async fn read_blocked_rows(database: &PostgresStore) -> Result<Vec<BlockedToolRow>, DbError> {
    let client = database
        .session()
        .await
        .map_err(|source| read_fault(&source))?;
    let rows = client
        .query(PERMISSIONS_SQL, &[])
        .await
        .map_err(|source| read_fault(&backend(source)))?;
    let mut permissions = Vec::with_capacity(rows.len());
    for row in &rows {
        let object_permission_id = row
            .try_get::<_, String>("object_permission_id")
            .map_err(|source| read_fault(&backend(source)))?;
        let blocked_tools = row
            .try_get::<_, Option<Vec<String>>>("blocked_tools")
            .map_err(|source| read_fault(&backend(source)))?;
        permissions.push(BlockedToolRow {
            object_permission_id,
            blocked_tools,
        });
    }
    Ok(permissions)
}

// The retry helper replays only transport faults, so a lost connection has to arrive as Connect.
fn read_fault(error: &NativeDbError) -> DbError {
    let mut message = error.to_string();
    let mut nested = error.source();
    while let Some(cause) = nested {
        message = format!("{message}: {cause}");
        nested = cause.source();
    }
    DbError {
        error_type: if is_native_transport_error(error) {
            ErrorType::Connect
        } else {
            ErrorType::Other
        },
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
