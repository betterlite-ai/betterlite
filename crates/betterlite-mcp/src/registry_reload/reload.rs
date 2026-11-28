use std::collections::BTreeSet;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_http::user_url::validate::Resolve;
use chrono::{DateTime, NaiveDateTime, Utc};
use indexmap::IndexMap;

use super::error::MutationError;
use super::hydrate::hydrate_config_servers;
use super::ports::{CacheInvalidator, ServerRows, ToolLister, ToolRegistrySink};
use super::prefix_owners::prefix_owners;
use super::{ENCRYPTED_DB_ENV_VARS, RegistryMutator};
use crate::config_loader::checks::config_ids_taken_by_db;
use crate::config_loader::ports::{DcrHydrator, InstructionsCache};
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_build::warnings::{
    warn_on_server_name_fields,
    // ast-grep-ignore: no-long-names
    warn_on_shared_identifier_prefixes,
};
use crate::server_build::{Decryptor, DeferredSlots, StdioPolicy};
use crate::server_oauth::trust::carry_forward_endpoints;
use crate::server_oauth::unresolved::endpoints_unresolved;
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
    pub async fn reload(&self) -> Result<(), MutationError> {
        tracing::debug!("Loading MCP servers from database into registry...");
        self.instructions.clear();
        let rows = self.rows.find_active().await?;
        tracing::info!("Found {} MCP servers in database", rows.len());
        let previous: IndexMap<String, MCPServer> = self.registry.read().await.db_servers.clone();
        let mut staged = self.stage_rows(&rows, &previous).await;
        let (registered, openapi_tools) = self.register_staged(&mut staged).await;
        self.publish(previous, registered, openapi_tools).await;
        Ok(())
    }

    async fn stage_rows(
        &self,
        rows: &[McpServerTable],
        previous: &IndexMap<String, MCPServer>,
    ) -> IndexMap<String, MCPServer> {
        let mut staged = IndexMap::with_capacity(rows.len());
        for row in rows {
            match self.stage_row(row, previous).await {
                Ok(server) => {
                    staged.insert(server.fields.server_id.clone(), server);
                }
                Err(error) => {
                    tracing::error!(
                        "Skipping MCP server {} ({}) during DB reload: {error}",
                        row.server_id,
                        row.alias.as_deref().unwrap_or("None")
                    );
                }
            }
        }
        staged
    }

    async fn stage_row(
        &self,
        row: &McpServerTable,
        previous: &IndexMap<String, MCPServer>,
    ) -> Result<MCPServer, MutationError> {
        let existing = previous.get(&row.server_id);
        if let Some(server) = existing.filter(|server| self.reusable(server, row)) {
            return Ok(server.clone());
        }
        warn_on_server_name_fields(
            &row.server_id,
            row.alias.as_deref(),
            row.server_name.as_deref(),
        );
        self.builder.warn_if_newly_blocked_stdio(row, existing);
        tracing::debug!(
            "Building server from DB: {} ({})",
            row.server_id,
            row.server_name.as_deref().unwrap_or("None")
        );
        let mut server = self
            .builder
            .build_from_table(row, ENCRYPTED_DB_ENV_VARS)
            .await?;
        if let Some(prefix) = existing
            .and_then(|server| server.fields.short_prefix.as_deref())
            .filter(|text| !text.is_empty())
        {
            server.fields.short_prefix = Some(prefix.to_owned());
        }
        carry_forward_endpoints(&mut server, existing);
        Ok(server)
    }

    fn reusable(&self, server: &MCPServer, row: &McpServerTable) -> bool {
        same_updated_at(server, row)
            && (self.slots.slot_exists(&row.server_id) || !endpoints_unresolved(server))
    }

    async fn register_staged(
        &self,
        staged: &mut IndexMap<String, MCPServer>,
    ) -> (IndexMap<String, MCPServer>, bool) {
        let mut registered = IndexMap::with_capacity(staged.len());
        let mut openapi_tools = false;
        let server_ids: Vec<String> = staged.keys().cloned().collect();
        for server_id in server_ids {
            let used = prefix_owners(staged.values(), &server_id);
            let Some(server) = staged.get_mut(&server_id) else {
                continue;
            };
            if let Err(error) = self.builder.assign_unique_short_prefix(server, &used) {
                log_skip(server, &MutationError::from(error));
                continue;
            }
            match self.register_openapi_tools(server).await {
                Ok(has_spec) => {
                    openapi_tools = openapi_tools || has_spec;
                    registered.insert(server_id, server.clone());
                }
                Err(error) => log_skip(server, &error),
            }
        }
        (registered, openapi_tools)
    }

    async fn publish(
        &self,
        previous: IndexMap<String, MCPServer>,
        registered: IndexMap<String, MCPServer>,
        openapi_tools: bool,
    ) {
        for (server_id, server) in &previous {
            if !registered.contains_key(server_id) {
                self.slots.invalidate_state(&server.fields.server_id);
            }
        }
        let mut ids: BTreeSet<&String> = previous.keys().collect();
        ids.extend(registered.keys());
        for server_id in ids {
            if previous.get(server_id) != registered.get(server_id) {
                self.caches.invalidate_server(server_id);
            }
        }
        let published: Vec<MCPServer> = registered.values().cloned().collect();
        self.registry.write().await.db_servers = registered;
        // ast-grep-ignore: no-long-names
        warn_on_shared_identifier_prefixes(&published);
        self.slots.reconcile(&published);
        self.slots.prime(&published);
        if openapi_tools {
            self.initialize_tool_owners();
        }
        tracing::debug!(
            "MCP registry refreshed ({} servers in registry)",
            published.len()
        );
        self.warn_reload_sets(&published).await;
        hydrate_config_servers(&self.registry, self.dcr).await;
    }

    async fn warn_reload_sets(&self, servers: &[MCPServer]) {
        let config_ids: BTreeSet<String> = self
            .registry
            .read()
            .await
            .config_servers
            .keys()
            .cloned()
            .collect();
        let registered_ids: BTreeSet<String> = servers
            .iter()
            .map(|server| server.fields.server_id.clone())
            .collect();
        let mut shadowed = BTreeSet::new();
        for server_id in &config_ids {
            if registered_ids.contains(server_id) {
                shadowed.insert(server_id.clone());
            }
        }
        if self.warned_shadowed.record(&shadowed) {
            tracing::warn!(
                "config.yaml MCP server_id(s) {} are also database-backed MCP servers. The database entry takes precedence, so the config.yaml server is unreachable. Give the config entry a different server_id.",
                joined_ids(&shadowed)
            );
        }
        let capturing = config_ids_taken_by_db(&config_ids, servers);
        if self.warned_capturing.record(&capturing) {
            tracing::warn!(
                "config.yaml MCP server_id(s) {} are the name or alias of a database-backed MCP server. Permission entries naming them resolve to the config.yaml server, not the database one. Give the config entry a different server_id.",
                joined_ids(&capturing)
            );
        }
    }
}

fn joined_ids(ids: &BTreeSet<String>) -> String {
    let values: Vec<&str> = ids.iter().map(String::as_str).collect();
    values.join(", ")
}

// both sides come from the same row text, so compare the instant the build step would store
fn same_updated_at(server: &MCPServer, row: &McpServerTable) -> bool {
    let (Some(previous), Some(current)) = (server.fields.updated_at, row.updated_at) else {
        return false;
    };
    let Ok(encoded) = serde_json::to_string(&current) else {
        return false;
    };
    if let Ok(aware) = serde_json::from_str::<DateTime<Utc>>(&encoded) {
        return aware == previous;
    }
    serde_json::from_str::<NaiveDateTime>(&encoded).is_ok_and(|naive| naive.and_utc() == previous)
}

fn log_skip(server: &MCPServer, error: &MutationError) {
    tracing::error!(
        "Skipping MCP server {} ({}) during DB reload: {error}",
        server.fields.server_id,
        server.fields.alias.as_deref().unwrap_or("None")
    );
}
