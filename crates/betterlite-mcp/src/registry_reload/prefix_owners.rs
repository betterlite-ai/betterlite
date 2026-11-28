use std::borrow::Borrow;
use std::collections::BTreeMap;

use crate::server_record::MCPServer;

pub(super) fn prefix_owners<I, T>(servers: I, server_id: &str) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = T>,
    T: Borrow<MCPServer>,
{
    let mut owners = BTreeMap::new();
    for candidate in servers {
        let server = candidate.borrow();
        if server.fields.server_id == server_id {
            continue;
        }
        let Some(prefix) = server
            .fields
            .short_prefix
            .as_deref()
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        owners.insert(prefix.to_owned(), server.fields.server_id.clone());
    }
    owners
}
