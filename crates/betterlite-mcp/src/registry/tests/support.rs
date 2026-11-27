use crate::registry::servers::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

pub(crate) fn settings(public: &[&str], ranges: Option<&[&str]>) -> RegistrySettings {
    let public_mcp_servers = public.iter().copied().map(String::from).collect();
    let internal_ip_ranges =
        ranges.map(|entries| entries.iter().copied().map(String::from).collect());
    RegistrySettings {
        public_mcp_servers,
        public_hub_strict: true,
        internal_ip_ranges,
    }
}

pub(crate) fn registry(config: &[&str], db: &[&str]) -> ServerRegistry {
    registry_with(settings(&[], None), config, db)
}

pub(crate) fn registry_with(
    settings: RegistrySettings,
    config: &[&str],
    db: &[&str],
) -> ServerRegistry {
    ServerRegistry::new(settings, records(config), records(db))
}

pub(crate) fn record(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn records(json: &[&str]) -> Vec<MCPServer> {
    json.iter().copied().map(record).collect()
}
