use crate::config_loader::ports::DcrHydrator;
use crate::registry::servers::SharedRegistry;
use crate::server_record::MCPServer;

// LiteLLM's _hydrate_config_servers_dcr_clients: best effort, never fails the reload
pub(super) async fn hydrate_config_servers<H>(registry: &SharedRegistry, dcr: &H)
where
    H: DcrHydrator,
{
    let config_servers: Vec<MCPServer> = {
        let held = registry.read().await;
        held.config_servers.values().cloned().collect()
    };
    let mut hydrated: Vec<MCPServer> = Vec::new();
    for mut server in config_servers {
        if dcr.hydrate(&mut server).await {
            hydrated.push(server);
        }
    }
    let mut held = registry.write().await;
    for server in &hydrated {
        let server_id = &server.fields.server_id;
        let Some(entry) = held.config_servers.get_mut(server_id) else {
            continue;
        };
        entry.clone_from(server);
        tracing::debug!(
            "hydrated persisted DCR client onto config MCP server server_id={server_id}"
        );
    }
}
