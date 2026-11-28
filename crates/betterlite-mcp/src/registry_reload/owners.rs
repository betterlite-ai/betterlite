use std::sync::{Arc, PoisonError};

use betterlite_http::user_url::validate::Resolve;

use super::RegistryMutator;
use super::ports::{
    CacheInvalidator, ServerRows, SharedToolOwners, ToolListError, ToolLister, ToolRegistrySink,
};
use crate::config_loader::ports::{DcrHydrator, InstructionsCache};
use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::registry::servers::SharedRegistry;
use crate::server_build::{Decryptor, DeferredSlots, StdioPolicy};
use crate::server_record::MCPServer;
use crate::server_utils::prefixes::{
    PrefixPolicy, iter_known_server_prefixes, match_known_server_prefix,
};
use crate::tools::schemas::McpTool;

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
    // LiteLLM's initialize_tool_name_to_mcp_server_name_mapping: fire and forget, never awaited
    pub fn initialize_tool_owners(&self) {
        self.tasks.spawn(
            ToolOwnerInitializer {
                registry: self.registry.clone(),
                slots: Arc::clone(&self.slots),
                owners: Arc::clone(&self.owners),
                lister: Arc::clone(&self.lister),
                policy: self.builder.prefixes.clone(),
            }
            .run(),
        );
    }
}

// the spawned body owns everything it reads, so it can outlive the mutator
struct ToolOwnerInitializer<F, C, L> {
    registry: SharedRegistry,
    slots: Arc<DiscoverySlots<F, C>>,
    owners: SharedToolOwners,
    lister: Arc<L>,
    policy: PrefixPolicy,
}

impl<F, C, L> ToolOwnerInitializer<F, C, L>
where
    F: MetadataFetcher + 'static,
    C: Clock + Send + Sync + 'static,
    L: ToolLister,
{
    async fn run(self) {
        let servers: Vec<MCPServer> = {
            let held = self.registry.read().await;
            held.get_registry().into_iter().cloned().collect()
        };
        for server in servers {
            if self.slots.slot_exists(&server.fields.server_id) {
                continue;
            }
            if server.needs_user_oauth_token() {
                continue;
            }
            match self.lister.list_tools(&server).await {
                Ok(tools) => record_tool_names(&self.owners, &server, &tools, &self.policy),
                Err(ToolListError::UpstreamAuth(error)) => tracing::debug!(
                    "Skipping tool name mapping for server {} due to upstream auth error: {error}",
                    server.fields.name
                ),
                Err(ToolListError::Failed(error)) => tracing::warn!(
                    "Failed to get tools from server {} during tool name mapping initialization: {error}",
                    server.fields.name
                ),
            }
        }
    }
}

fn record_tool_names(
    owners: &SharedToolOwners,
    server: &MCPServer,
    tools: &[McpTool],
    policy: &PrefixPolicy,
) {
    let known_prefixes = iter_known_server_prefixes(server, policy);
    let mut held = owners.write().unwrap_or_else(PoisonError::into_inner);
    for tool in tools {
        let original_name = match match_known_server_prefix(&tool.name, &known_prefixes, policy) {
            Some((_matched_prefix, bare_name)) => bare_name,
            None => tool.name.clone(),
        };
        held.mapping
            .insert(original_name, server.fields.name.clone());
        held.mapping
            .insert(tool.name.clone(), server.fields.name.clone());
    }
}
