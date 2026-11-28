use std::collections::BTreeSet;
use std::sync::PoisonError;

use betterlite_http::user_url::validate::Resolve;

use super::RegistryMutator;
use super::ports::{CacheInvalidator, ServerRows, ToolLister, ToolRegistrySink};
use crate::config_loader::ports::{DcrHydrator, InstructionsCache};
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_build::{Decryptor, DeferredSlots, StdioPolicy};
use crate::server_record::MCPServer;
use crate::server_utils::prefixes::{get_server_prefix, iter_known_server_prefixes};

// LiteLLM's _cleanup_server_tool_routing_artifacts and _owned_mapping_values, on a trait because the workspace denies multiple_inherent_impl
pub trait ToolRoutingCleanup {
    fn cleanup_tool_routing_artifacts(&self, server: &MCPServer);

    fn owned_mapping_values(&self, server: &MCPServer) -> BTreeSet<String>;
}

impl<D, F, S, R, C, Rows, H, L, T, I, X, U> ToolRoutingCleanup
    for RegistryMutator<'_, D, F, S, R, C, Rows, H, L, T, I, X, U>
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
    // LiteLLM's _cleanup_server_tool_routing_artifacts
    fn cleanup_tool_routing_artifacts(&self, server: &MCPServer) {
        self.caches.invalidate_server(&server.fields.server_id);
        let prefix_root = get_server_prefix(server, &self.builder.prefixes).replace(' ', "_");
        let has_spec = server
            .fields
            .spec_path
            .as_deref()
            .is_some_and(|path| !path.is_empty());
        if has_spec && !prefix_root.is_empty() {
            let separator = &self.builder.prefixes.separator;
            self.tools
                .unregister_prefix(&format!("{prefix_root}{separator}"));
        }
        let owned = self.owned_mapping_values(server);
        let mut owners = self.owners.write().unwrap_or_else(PoisonError::into_inner);
        owners
            .mapping
            .retain(|_tool_name, mapped| !owned.contains(&mapped.replace(' ', "_")));
    }

    // LiteLLM's _owned_mapping_values: every mapping value a tool of this server may carry
    fn owned_mapping_values(&self, server: &MCPServer) -> BTreeSet<String> {
        // iter_known_server_prefixes already drops empty prefixes, so no filter is needed here
        let mut owned: BTreeSet<String> =
            iter_known_server_prefixes(server, &self.builder.prefixes)
                .into_iter()
                .map(|prefix| prefix.replace(' ', "_"))
                .collect();
        let name = server.fields.name.replace(' ', "_");
        if !name.is_empty() {
            owned.insert(name);
        }
        owned
    }
}
