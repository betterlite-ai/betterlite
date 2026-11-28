use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_http::user_url::validate::Resolve;
use indexmap::IndexMap;

use super::error::MutationError;
use super::ports::{CacheInvalidator, ServerRows, ToolLister, ToolRegistrySink};
use super::prefix_owners::prefix_owners;
use super::{PLAIN_DB_ENV_VARS, RegistryMutator};
use crate::config_loader::ports::{DcrHydrator, InstructionsCache};
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_build::{Decryptor, DeferredSlots, StdioPolicy};
use crate::server_oauth::trust::carry_forward_endpoints;
use crate::server_record::MCPServer;

impl<D, F, S, R, C, Rows, H, L, T, I, X, U>
    RegistryMutator<'_, D, F, S, R, C, Rows, H, L, T, I, X, U>
where
    D: Decryptor,
    F: MetadataFetcher + 'static,
    S: StdioPolicy,
    R: DeferredSlots,
    C: Clock + Send + Sync + 'static,
    Rows: ServerRows,
    H: DcrHydrator,
    L: ToolLister + 'static,
    T: ToolRegistrySink,
    I: InstructionsCache,
    X: CacheInvalidator,
    U: Resolve,
{
    // LiteLLM's add_server
    pub async fn add(&self, row: &McpServerTable) -> Result<(), MutationError> {
        if !eligible(row) {
            return Ok(());
        }
        let result = self.add_eligible(row).await;
        if let Err(error) = &result {
            tracing::debug!("Failed to add MCP server: {error}");
        }
        result
    }

    async fn add_eligible(&self, row: &McpServerTable) -> Result<(), MutationError> {
        if self
            .registry
            .read()
            .await
            .db_servers
            .contains_key(&row.server_id)
        {
            return Ok(());
        }
        self.builder.warn_if_newly_blocked_stdio(row, None);
        let mut server = self
            .builder
            .build_from_table(row, PLAIN_DB_ENV_VARS)
            .await?;
        let used = self.used_prefixes(&server.fields.server_id).await;
        self.builder
            .assign_unique_short_prefix(&mut server, &used)?;
        self.caches.invalidate_server(&row.server_id);
        self.registry
            .write()
            .await
            .db_servers
            .insert(row.server_id.clone(), server.clone());
        if self.register_openapi_tools(&server).await? {
            self.initialize_tool_owners();
        }
        drop(self.slots.prime_one(&server));
        tracing::debug!("Added MCP Server: {}", server.fields.name);
        Ok(())
    }

    // LiteLLM's update_server
    pub async fn update(&self, row: &McpServerTable) -> Result<(), MutationError> {
        if !eligible(row) {
            let evicted = {
                let mut held = self.registry.write().await;
                pop_registered(&mut held.db_servers, row)
            };
            if let Some(server) = evicted {
                self.cleanup_tool_routing_artifacts(&server);
                self.slots.invalidate_state(&server.fields.server_id);
            }
            return Ok(());
        }
        let result = self.update_eligible(row).await;
        if let Err(error) = &result {
            tracing::debug!("Failed to udpate MCP server: {error}");
        }
        result
    }

    async fn update_eligible(&self, row: &McpServerTable) -> Result<(), MutationError> {
        let existing = self
            .registry
            .read()
            .await
            .db_servers
            .get(&row.server_id)
            .cloned();
        let Some(previous) = existing else {
            return Ok(());
        };
        let mut server = self
            .builder
            .build_from_table(row, PLAIN_DB_ENV_VARS)
            .await?;
        if server
            .fields
            .short_prefix
            .as_deref()
            .is_none_or(str::is_empty)
        {
            if let Some(prefix) = previous
                .fields
                .short_prefix
                .as_deref()
                .filter(|prefix| !prefix.is_empty())
            {
                server.fields.short_prefix = Some(prefix.to_owned());
            }
        }
        carry_forward_endpoints(&mut server, Some(&previous));
        let used = self.used_prefixes(&server.fields.server_id).await;
        self.builder
            .assign_unique_short_prefix(&mut server, &used)?;
        self.caches.invalidate_server(&row.server_id);
        self.registry
            .write()
            .await
            .db_servers
            .insert(row.server_id.clone(), server.clone());
        if self.register_openapi_tools(&server).await? {
            self.initialize_tool_owners();
        }
        drop(self.slots.prime_one(&server));
        tracing::debug!("Updated MCP Server: {}", server.fields.name);
        Ok(())
    }

    // LiteLLM's remove_server
    pub async fn remove(&self, row: &McpServerTable) {
        let evicted = {
            let mut held = self.registry.write().await;
            pop_registered(&mut held.db_servers, row)
        };
        if let Some(server) = evicted {
            tracing::debug!("Removed MCP Server: {}", removal_label(row));
            self.cleanup_tool_routing_artifacts(&server);
            self.slots.invalidate_state(&server.fields.server_id);
        } else {
            tracing::warn!("Server ID {} not found in registry", row.server_id);
        }
    }

    async fn used_prefixes(&self, server_id: &str) -> BTreeMap<String, String> {
        let held = self.registry.read().await;
        prefix_owners(held.get_registry(), server_id)
    }
}

fn eligible(row: &McpServerTable) -> bool {
    matches!(
        row.approval_status.as_deref(),
        None | Some("active" | "approved")
    )
}

fn pop_registered(
    servers: &mut IndexMap<String, MCPServer>,
    row: &McpServerTable,
) -> Option<MCPServer> {
    if let Some(server) = servers.shift_remove(&row.server_id) {
        return Some(server);
    }
    let name = row.server_name.as_deref().filter(|name| !name.is_empty())?;
    servers.shift_remove(name)
}

fn removal_label(row: &McpServerTable) -> &str {
    if row.server_id.is_empty() {
        return row.server_name.as_deref().unwrap_or("None");
    }
    &row.server_id
}
