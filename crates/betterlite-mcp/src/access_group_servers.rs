use std::error::Error as StdError;
use std::future::Future;

use betterlite_db::mcp::table_access::store::McpTableStore;
use betterlite_db::mcp::table_access::types::McpTableError;
use thiserror::Error;
use tracing::debug;

use crate::registry::servers::{ServerRegistry, SharedRegistry};

pub trait AccessGroupServerRows: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    // ast-grep-ignore: no-long-names
    fn server_ids_with_any_access_group(
        &self,
        access_groups: &[String],
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send;
}

impl AccessGroupServerRows for McpTableStore<'_> {
    type Error = McpTableError;

    // ast-grep-ignore: no-long-names
    fn server_ids_with_any_access_group(
        &self,
        access_groups: &[String],
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send {
        async move {
            // ast-grep-ignore: no-long-names
            <McpTableStore<'_>>::server_ids_with_any_access_group(self, access_groups, use_writer)
                .await
        }
    }
}

#[derive(Debug, Error)]
pub enum AccessGroupServersError<RowError>
where
    RowError: StdError + Send + Sync + 'static,
{
    #[error("the MCP servers for the access groups could not be read")]
    Rows(#[source] RowError),
}

// LiteLLM's _get_config_server_ids_for_access_groups: only config servers, and only a non-empty group list can match
#[must_use]
// ast-grep-ignore: no-long-names
pub fn config_server_ids_for_access_groups(
    registry: &ServerRegistry,
    access_groups: &[String],
) -> Vec<String> {
    let mut server_ids = Vec::new();
    for (server_id, server) in &registry.config_servers {
        let Some(groups) = server.fields.access_groups.as_ref() else {
            continue;
        };
        if groups.is_empty() {
            continue;
        }
        if groups.iter().any(|group| access_groups.contains(group)) {
            server_ids.push(server_id.clone());
        }
    }
    server_ids
}

// ast-grep-ignore: no-long-names - LiteLLM's _get_db_server_ids_for_access_groups swallows a fault at debug unless the caller needs the writer
pub async fn db_server_ids_for_access_groups<R: AccessGroupServerRows>(
    rows: Option<&R>,
    access_groups: &[String],
    use_writer: bool,
) -> Result<Vec<String>, AccessGroupServersError<R::Error>> {
    let Some(rows) = rows else {
        return Ok(Vec::new());
    };
    if access_groups.is_empty() {
        return Ok(Vec::new());
    }
    match rows
        .server_ids_with_any_access_group(access_groups, use_writer)
        .await
    {
        Ok(server_ids) => Ok(server_ids),
        Err(error) => {
            if use_writer {
                Err(AccessGroupServersError::Rows(error))
            } else {
                debug!("Error getting MCP servers from access groups: {error}");
                Ok(Vec::new())
            }
        }
    }
}

// LiteLLM's _get_mcp_servers_from_access_groups: the config ids first, then the database ids not already listed
pub async fn servers_from_access_groups<R: AccessGroupServerRows>(
    registry: &SharedRegistry,
    rows: Option<&R>,
    access_groups: &[String],
    requires_fresh_policy: bool,
) -> Result<Vec<String>, AccessGroupServersError<R::Error>> {
    let config_ids = {
        let guard = registry.read().await;
        // ast-grep-ignore: no-long-names
        config_server_ids_for_access_groups(&guard, access_groups)
    };
    let db_ids = {
        // ast-grep-ignore: no-long-names
        db_server_ids_for_access_groups(rows, access_groups, requires_fresh_policy).await?
    };
    let mut server_ids = config_ids;
    for server_id in db_ids {
        if !server_ids.contains(&server_id) {
            server_ids.push(server_id);
        }
    }
    Ok(server_ids)
}

mod tests;
