use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::MCPTransport;

use super::{BuildError, ServerBuilder, StdioPolicy};
use crate::server_payload::rules::MCP_STDIO_DISABLED_MESSAGE;
use crate::server_record::MCPServer;
use crate::server_utils::prefixes::compute_short_server_prefix;

const MAX_REHASH_ATTEMPTS: u32 = 1024;

impl<D, F, S, R> ServerBuilder<'_, D, F, S, R> {
    pub fn assign_unique_short_prefix(
        &self,
        server: &mut MCPServer,
        used_prefixes: &BTreeMap<String, String>,
    ) -> Result<(), BuildError> {
        if !self.prefixes.short_prefix_enabled
            || server
                .fields
                .short_prefix
                .as_deref()
                .is_some_and(|prefix| !prefix.is_empty())
            || server.fields.server_id.is_empty()
        {
            return Ok(());
        }
        for attempt in 0..MAX_REHASH_ATTEMPTS {
            let candidate = compute_short_server_prefix(&server.fields.server_id, attempt)
                .map_err(|source| BuildError::Prefix {
                    server_id: server.fields.server_id.clone(),
                    source,
                })?;
            if used_prefixes
                .get(&candidate)
                .is_some_and(|owner| owner.as_str() != server.fields.server_id)
            {
                continue;
            }
            if attempt > 0 {
                log_rehash(server, used_prefixes, &candidate, attempt);
            }
            server.fields.short_prefix = Some(candidate);
            return Ok(());
        }
        Err(BuildError::PrefixExhausted {
            server_id: server.fields.server_id.clone(),
            attempts: MAX_REHASH_ATTEMPTS,
        })
    }
}

impl<D, F, S, R> ServerBuilder<'_, D, F, S, R>
where
    S: StdioPolicy,
{
    pub fn warn_if_newly_blocked_stdio(&self, row: &McpServerTable, previous: Option<&MCPServer>) {
        if previous.is_some_and(|older| older.fields.transport == row.transport) {
            return;
        }
        if row.transport != MCPTransport::Stdio || !self.stdio.is_blocked() {
            return;
        }
        let name = row
            .alias
            .as_deref()
            .filter(|text| !text.is_empty())
            .or(row.server_name.as_deref().filter(|text| !text.is_empty()));
        tracing::warn!(
            "MCP server '{}' will not start: {MCP_STDIO_DISABLED_MESSAGE}",
            name.unwrap_or("None")
        );
    }
}

fn log_rehash(
    server: &MCPServer,
    used_prefixes: &BTreeMap<String, String>,
    candidate: &str,
    attempt: u32,
) {
    let natural = compute_short_server_prefix(&server.fields.server_id, 0).ok();
    let owner = natural
        .as_deref()
        .and_then(|prefix| used_prefixes.get(prefix))
        .map_or("<unknown>", String::as_str);
    tracing::info!(
        "MCP short-prefix collision resolved for server {}: natural hash collided with {}, using rehashed prefix {} (attempt={}).",
        server.fields.server_id,
        owner,
        candidate,
        attempt
    );
}
