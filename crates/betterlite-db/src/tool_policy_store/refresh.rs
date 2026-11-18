use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::timeout;

use crate::database_faults::native::{is_native_transport_error, native_sqlstate};
use crate::database_faults::reconnect::{DbReconnect, RetryPolicy, call_with_db_reconnect_retry};
use crate::database_faults::{DbError, ErrorType};
use crate::postgres_store::errors::{DbError as NativeDbError, backend};
use crate::postgres_store::session::PostgresStore;
use crate::tool_records::decode::decode_tool_record;
use crate::tool_records::records::LiteLLMToolTableRow;

use super::deadline::{StatementBounds, StatementFailure, bounded_statement, discard_uncancelled};
use super::errors::ToolPolicyStoreError;
use super::records::BlockedToolRow;
use super::registry::{ToolPolicyMaps, ToolPolicyRegistry, blocked_tools_map, tool_policy_maps};

const TOOLS_SQL: &str =
    "SELECT row_to_json(entry)::text AS row_text FROM \"LiteLLM_ToolTable\" AS entry";
const PERMISSIONS_SQL: &str =
    "SELECT \"object_permission_id\", \"blocked_tools\" FROM \"LiteLLM_ObjectPermissionTable\"";
const TOOLS_REASON: &str = "sync_tool_policy_from_db_tools_lookup_failure";
const PERMISSIONS_REASON: &str = "sync_tool_policy_from_db_perms_lookup_failure";
const TOOLS_LOOKUP: &str = "tool policy";
const PERMISSIONS_LOOKUP: &str = "object permission blocked_tools";

pub struct ToolPolicyStore<'store> {
    pub database: &'store PostgresStore,
    pub registry: &'store Mutex<ToolPolicyRegistry>,
    pub bounds: StatementBounds<'store>,
}

enum ReadFault {
    Native(NativeDbError),
    Expired(&'static str),
}

impl ReadFault {
    fn classify(&self, deadline: Duration) -> DbError {
        match self {
            Self::Native(source) => DbError {
                error_type: error_type_of(source),
                message: source.to_string(),
                code: native_sqlstate(source).map(|code| code.code().to_owned()),
                meta_code: None,
                traceback_modules: Vec::new(),
                cause: None,
            },
            Self::Expired(lookup) => DbError {
                error_type: ErrorType::Deadline,
                message: format!("the {lookup} lookup did not finish within {deadline:?}"),
                code: None,
                meta_code: None,
                traceback_modules: Vec::new(),
                cause: None,
            },
        }
    }

    fn into_store_error(
        self,
        deadline: Duration,
        reconnect: Option<Box<DbError>>,
    ) -> ToolPolicyStoreError {
        match self {
            Self::Native(source) => ToolPolicyStoreError::Read {
                error_type: error_type_of(&source),
                source,
                reconnect,
            },
            Self::Expired(lookup) => ToolPolicyStoreError::Deadline { lookup, deadline },
        }
    }
}

fn error_type_of(source: &NativeDbError) -> ErrorType {
    if is_native_transport_error(source) {
        ErrorType::Connect
    } else {
        ErrorType::Other
    }
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
    let texts = load_rows(
        reconnect.as_deref_mut(),
        store.bounds,
        TOOLS_REASON,
        move || read_tool_texts(store.database, store.bounds),
    )
    .await?;
    let maps = tool_policy_maps(&decode_tools(&texts)?);
    let tool_count = maps.input_policies.len();
    install_tool_maps(store.registry, maps);
    let blocked_tools = load_blocked_tools(store.database, reconnect, store.bounds).await?;
    let permission_count = blocked_tools.len();
    let mut registry = store
        .registry
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    registry.blocked_tools_by_permission = blocked_tools;
    registry.initialized = true;
    drop(registry);
    tracing::info!(
        "ToolPolicyRegistry: synced {tool_count} tool policies and {permission_count} object permissions from DB"
    );
    Ok(())
}

fn install_tool_maps(registry: &Mutex<ToolPolicyRegistry>, maps: ToolPolicyMaps) {
    let mut state = registry.lock().unwrap_or_else(PoisonError::into_inner);
    state.input_policies = maps.input_policies;
    state.output_policies = maps.output_policies;
}

fn decode_tools(texts: &[String]) -> Result<Vec<LiteLLMToolTableRow>, ToolPolicyStoreError> {
    let mut tools = Vec::with_capacity(texts.len());
    for text in texts {
        tools.push(
            decode_tool_record(text.as_bytes())
                .map_err(|source| ToolPolicyStoreError::Record { source })?,
        );
    }
    Ok(tools)
}

pub(super) async fn load_blocked_tools<Client>(
    database: &PostgresStore,
    reconnect: Option<&mut Client>,
    bounds: StatementBounds<'_>,
) -> Result<BTreeMap<String, Vec<String>>, ToolPolicyStoreError>
where
    Client: DbReconnect + Send,
{
    let rows = load_rows(reconnect, bounds, PERMISSIONS_REASON, move || {
        read_rows(
            database,
            PERMISSIONS_SQL,
            PERMISSIONS_LOOKUP,
            bounds,
            blocked_tool_row,
        )
    })
    .await?;
    Ok(blocked_tools_map(&rows))
}

async fn load_rows<Rows, Read, ReadFuture, Client>(
    reconnect: Option<&mut Client>,
    bounds: StatementBounds<'_>,
    reason: &'static str,
    read: Read,
) -> Result<Vec<Rows>, ToolPolicyStoreError>
where
    Client: DbReconnect + Send,
    Rows: Send,
    Read: Fn() -> ReadFuture + Send + Sync,
    ReadFuture: Future<Output = Result<Vec<Rows>, ReadFault>> + Send,
{
    let fault: Mutex<Option<ReadFault>> = Mutex::new(None);
    let rows = {
        let last_fault = &fault;
        let reader = &read;
        call_with_db_reconnect_retry(
            reconnect,
            move || async move {
                match reader().await {
                    Ok(rows) => Ok(rows),
                    Err(failure) => {
                        let classified = failure.classify(bounds.deadline);
                        *last_fault.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
                        Err(classified)
                    }
                }
            },
            RetryPolicy {
                reason,
                retry_safe_error_types: None,
                timeout_seconds: None,
                lock_timeout_seconds: None,
            },
        )
        .await
    };
    rows.map_err(|classified| read_failure(fault, classified, bounds.deadline))
}

fn read_failure(
    last_fault: Mutex<Option<ReadFault>>,
    classified: DbError,
    deadline: Duration,
) -> ToolPolicyStoreError {
    match last_fault
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
    {
        Some(fault) => fault.into_store_error(deadline, classified.cause),
        None => ToolPolicyStoreError::ClassifiedRead { source: classified },
    }
}

async fn read_rows<T, Map>(
    database: &PostgresStore,
    sql: &str,
    lookup: &'static str,
    bounds: StatementBounds<'_>,
    map: Map,
) -> Result<Vec<T>, ReadFault>
where
    T: Send,
    Map: Fn(&tokio_postgres::Row) -> Result<T, ReadFault> + Send,
{
    let client = match timeout(bounds.deadline, database.session()).await {
        Ok(Ok(client)) => client,
        Ok(Err(source)) => return Err(ReadFault::Native(source)),
        Err(_elapsed) => return Err(ReadFault::Expired(lookup)),
    };
    match bounded_statement(&client, bounds, client.query(sql, &[])).await {
        Ok(rows) => rows.iter().map(map).collect(),
        Err(StatementFailure::Driver(source)) => Err(ReadFault::Native(backend(source))),
        Err(failure @ StatementFailure::Expired { .. }) => {
            discard_uncancelled(client, &failure);
            Err(ReadFault::Expired(lookup))
        }
    }
}

async fn read_tool_texts(
    database: &PostgresStore,
    bounds: StatementBounds<'_>,
) -> Result<Vec<String>, ReadFault> {
    read_rows(database, TOOLS_SQL, TOOLS_LOOKUP, bounds, |row| {
        row.try_get::<_, String>("row_text")
            .map_err(|source| ReadFault::Native(backend(source)))
    })
    .await
}

fn blocked_tool_row(row: &tokio_postgres::Row) -> Result<BlockedToolRow, ReadFault> {
    let object_permission_id = row
        .try_get::<_, String>("object_permission_id")
        .map_err(|source| ReadFault::Native(backend(source)))?;
    let blocked_tools = row
        .try_get::<_, Option<Vec<String>>>("blocked_tools")
        .map_err(|source| ReadFault::Native(backend(source)))?;
    Ok(BlockedToolRow {
        object_permission_id,
        blocked_tools,
    })
}
