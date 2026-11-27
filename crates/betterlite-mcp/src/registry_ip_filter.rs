use crate::registry::lookup::ServerLookups as _;
use crate::registry::servers::ServerRegistry;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpFilteredIds {
    pub server_ids: Vec<String>,
    pub ip_blocked_count: usize,
}

#[must_use]
// ast-grep-ignore: no-long-names - the reference names this split, and B4726 and B8195 read the blocked count
pub fn filter_server_ids_by_ip_with_info(
    registry: &ServerRegistry,
    server_ids: &[String],
    client_ip: Option<&str>,
) -> IpFilteredIds {
    let Some(client_address) = client_ip else {
        return IpFilteredIds {
            server_ids: server_ids.to_vec(),
            ip_blocked_count: 0,
        };
    };
    let mut allowed_server_ids = Vec::with_capacity(server_ids.len());
    let mut ip_blocked_count = 0;
    for server_id in server_ids {
        let Some(server) = registry.server_by_id(server_id, None) else {
            continue;
        };
        if registry.is_accessible_from_ip(server, Some(client_address)) {
            allowed_server_ids.push(server_id.clone());
        } else {
            ip_blocked_count = ip_blocked_count.saturating_add(1);
        }
    }
    IpFilteredIds {
        server_ids: allowed_server_ids,
        ip_blocked_count,
    }
}

#[must_use]
pub fn filter_server_ids_by_ip(
    registry: &ServerRegistry,
    server_ids: &[String],
    client_ip: Option<&str>,
) -> Vec<String> {
    // ast-grep-ignore: no-long-names - the with-info sibling above declares this name
    filter_server_ids_by_ip_with_info(registry, server_ids, client_ip).server_ids
}

mod tests;
