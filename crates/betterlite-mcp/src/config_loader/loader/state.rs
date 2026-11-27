use std::collections::BTreeMap;
use std::sync::Arc;

use super::super::dump::redacted_registry_dump;
use super::super::ports::DcrHydrator;
use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::registry::servers::SharedRegistry;
use crate::server_record::MCPServer;

pub(super) async fn hydrate_dcr_clients<H: DcrHydrator>(registry: &SharedRegistry, dcr: &H) {
    let server_ids: Vec<String> = registry
        .read()
        .await
        .config_servers
        .keys()
        .cloned()
        .collect();
    for server_id in server_ids {
        let stored = {
            let held = registry.read().await;
            held.config_servers.get(&server_id).cloned()
        };
        let Some(mut server) = stored else {
            continue;
        };
        if !dcr.hydrate(&mut server).await {
            continue;
        }
        {
            let mut table = registry.write().await;
            if let Some(entry) = table.config_servers.get_mut(&server_id) {
                entry.clone_from(&server);
            }
        }
        tracing::debug!(
            "hydrated persisted DCR client onto config MCP server server_id={server_id}"
        );
    }
}

pub(super) async fn used_prefixes(
    registry: &SharedRegistry,
    server_id: &str,
) -> BTreeMap<String, String> {
    let held = registry.read().await;
    let mut used = BTreeMap::new();
    let servers = held.config_servers.values().chain(held.db_servers.values());
    for server in servers {
        if server.fields.server_id == server_id {
            continue;
        }
        if let Some(prefix) = server
            .fields
            .short_prefix
            .as_ref()
            .filter(|prefix| !prefix.is_empty())
        {
            used.insert(prefix.clone(), server.fields.server_id.clone());
        }
    }
    used
}

pub(super) async fn log_redacted_dump(registry: &SharedRegistry) {
    let dump = redacted_registry_dump(&registry.read().await.config_servers);
    let text = serde_json::to_string_pretty(&dump).unwrap_or_default();
    tracing::debug!("Loaded MCP Servers: {text}");
}

pub(super) async fn prime_discovery<F, C>(
    registry: &SharedRegistry,
    slots: &Arc<DiscoverySlots<F, C>>,
) where
    F: MetadataFetcher + 'static,
    C: Clock + 'static,
{
    let servers: Vec<MCPServer> = registry
        .read()
        .await
        .config_servers
        .values()
        .cloned()
        .collect();
    slots.prime(&servers);
}
