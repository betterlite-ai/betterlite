use std::collections::BTreeSet;

use betterlite_db::mcp_stored_contracts::MCPTransport;
use indexmap::IndexMap;

use super::super::checks::{CollisionCheck, reject_id_collision};
use super::super::config::McpServerConfig;
use super::super::error::ConfigError;
use super::super::id::{generate_stable_server_id, pinned_config_server_id};
use super::super::plan::OAuthPlan;
use super::{Entry, LoadState, Prepared};
use crate::server_build::warnings::warn_on_server_name_fields;
use crate::server_oauth::warnings::{UnresolvedCheck, warn_unresolved};
use crate::server_payload::rules::MCP_STDIO_DISABLED_MESSAGE;
use crate::server_record::MCPOAuthMetadata;

// get_server_prefix on a temporary server: the alias when non-empty, else the entry's name
pub(super) fn display_name(server_name: &str, alias: Option<&str>) -> String {
    match alias {
        Some(candidate) if !candidate.is_empty() => candidate.to_owned(),
        Some(_) | None => server_name.to_owned(),
    }
}

pub(super) fn resolve_server_id(
    entry: Entry<'_>,
    alias: Option<&str>,
    server_url: &str,
    state: &mut LoadState,
) -> Result<String, ConfigError> {
    let config = entry.config;
    let pinned = pinned_config_server_id(config.server_id.as_ref(), entry.server_name)?;
    let server_id = pinned.as_ref().map_or_else(
        || {
            generate_stable_server_id(
                entry.server_name,
                server_url,
                config.transport,
                config.auth_type,
                alias,
            )
        },
        Clone::clone,
    );
    reject_id_collision(CollisionCheck {
        assigned_server_ids: &state.assigned_server_ids,
        server_id: &server_id,
        server_name: entry.server_name,
        pinned: pinned.is_some(),
        db_backed_server_ids: &state.db_backed_server_ids,
        identifier_owners: &state.identifier_owners,
    })?;
    state
        .assigned_server_ids
        .insert(server_id.clone(), entry.server_name.to_owned());
    warn_on_server_name_fields(&server_id, alias, Some(entry.server_name));
    Ok(server_id)
}

pub(super) fn select_alias(
    server_name: &str,
    config: &McpServerConfig,
    mcp_aliases: Option<&IndexMap<String, String>>,
    used_aliases: &mut BTreeSet<String>,
) -> Option<String> {
    if let Some(alias) = config.alias.as_ref() {
        return Some(alias.clone());
    }
    let aliases = mcp_aliases?;
    for (alias_name, target_server_name) in aliases {
        if target_server_name == server_name && !used_aliases.contains(alias_name) {
            used_aliases.insert(alias_name.clone());
            tracing::debug!("Mapped alias '{alias_name}' to server '{server_name}'");
            return Some(alias_name.clone());
        }
    }
    None
}

pub(super) fn warn_stdio_blocked(
    server_name: &str,
    transport: Option<MCPTransport>,
    blocked: bool,
) {
    if transport != Some(MCPTransport::Stdio) || !blocked {
        return;
    }
    tracing::warn!("MCP server '{server_name}' will not start: {MCP_STDIO_DISABLED_MESSAGE}");
}

pub(super) fn warn_oauth_unresolved(
    prepared: &Prepared,
    plan: &OAuthPlan,
    metadata: Option<&MCPOAuthMetadata>,
) {
    warn_unresolved(UnresolvedCheck {
        server_ref: &prepared.server_ref,
        server_url: Some(&prepared.server_url),
        discovery_attempted: plan.should_discover,
        discovery_deferred: plan.deferred,
        issuer_anchored: plan.use_issuer_anchor,
        metadata,
        needs_authorization_url: plan.needs_authorization_url,
        needs_token_url: plan.needs_token_url,
        manual_authorization_url: plan.endpoints.manual_authorization.as_deref(),
        manual_token_url: plan.endpoints.manual_token.as_deref(),
    });
}
