use std::collections::BTreeSet;

use betterlite_mcp::registry::servers::ServerRegistry;

#[must_use]
pub fn allow_all_keys_server_ids(registry: &ServerRegistry) -> BTreeSet<String> {
    registry
        .get_allow_all_keys_server_ids()
        .into_iter()
        .collect()
}
