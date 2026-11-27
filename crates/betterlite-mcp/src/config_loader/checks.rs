use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::config::McpServerConfig;
use super::error::ConfigError;
use super::values::python_repr;
use crate::server_payload::names::validate_mcp_server_name;
use crate::server_payload::request::is_per_server_oauth_discovery_eligible;
use crate::server_record::{MCPServer, OAuth2Flow, ServerFlag};

pub fn validate_config_server_names(
    config: &IndexMap<String, McpServerConfig>,
    separator: &str,
) -> Result<(), ConfigError> {
    for server_name in config.keys() {
        validate_mcp_server_name(server_name, separator)?;
    }
    Ok(())
}

#[must_use]
pub fn first_mapped_alias(
    server_name: &str,
    mcp_aliases: Option<&IndexMap<String, String>>,
) -> Option<String> {
    let aliases = mcp_aliases?;
    for (alias_name, target_server_name) in aliases {
        if target_server_name == server_name {
            return Some(alias_name.clone());
        }
    }
    None
}

#[must_use]
pub fn assigned_alias(
    server_name: &str,
    config: &McpServerConfig,
    mcp_aliases: Option<&IndexMap<String, String>>,
) -> Option<String> {
    match config.alias.as_ref() {
        Some(alias) => Some(alias.clone()),
        None => first_mapped_alias(server_name, mcp_aliases),
    }
}

#[must_use]
pub fn config_identifier_owners(
    config: &IndexMap<String, McpServerConfig>,
    mcp_aliases: Option<&IndexMap<String, String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (server_name, server_config) in config {
        let alias = assigned_alias(server_name, server_config, mcp_aliases);
        let claims = [Some(server_name.as_str()), alias.as_deref()];
        for identifier in claims.into_iter().flatten() {
            if identifier.is_empty() {
                continue;
            }
            owners
                .entry(identifier.to_owned())
                .or_default()
                .insert(server_name.clone());
        }
    }
    owners
}

#[must_use]
pub fn config_ids_capturing_db_identifiers(
    config_server_ids: &BTreeSet<String>,
    db_servers: &[MCPServer],
) -> BTreeSet<String> {
    let mut captured = BTreeSet::new();
    for server in db_servers {
        let identifiers = [
            Some(server.fields.name.as_str()),
            server.fields.server_name.as_deref(),
            server.fields.alias.as_deref(),
        ];
        for identifier in identifiers.into_iter().flatten() {
            if identifier.is_empty() || identifier == server.fields.server_id {
                continue;
            }
            if config_server_ids.contains(identifier) {
                captured.insert(identifier.to_owned());
            }
        }
    }
    captured
}

#[derive(Clone, Copy)]
pub struct CollisionCheck<'check> {
    pub assigned_server_ids: &'check BTreeMap<String, String>,
    pub server_id: &'check str,
    pub server_name: &'check str,
    pub pinned: ServerFlag,
    pub db_backed_server_ids: &'check BTreeSet<String>,
    pub identifier_owners: &'check BTreeMap<String, BTreeSet<String>>,
}

pub fn reject_config_server_id_collision(check: CollisionCheck<'_>) -> Result<(), ConfigError> {
    if let Some(claimed_by) = check.assigned_server_ids.get(check.server_id) {
        return Err(ConfigError::CollisionClaimed {
            server_name: check.server_name.to_owned(),
            server_id: check.server_id.to_owned(),
            claimed_by: claimed_by.clone(),
        });
    }
    if check.pinned && check.db_backed_server_ids.contains(check.server_id) {
        return Err(ConfigError::CollisionDatabase {
            server_name: check.server_name.to_owned(),
            server_id: check.server_id.to_owned(),
        });
    }
    let no_owners = BTreeSet::new();
    let owners = check
        .identifier_owners
        .get(check.server_id)
        .unwrap_or(&no_owners);
    let other_owners: Vec<&str> = owners
        .iter()
        .map(String::as_str)
        .filter(|owner| *owner != check.server_name)
        .collect();
    if check.pinned && !other_owners.is_empty() {
        return Err(ConfigError::CollisionIdentifier {
            server_name: check.server_name.to_owned(),
            server_id: check.server_id.to_owned(),
            owners: other_owners.join("', '"),
        });
    }
    Ok(())
}

pub fn dcr_bridge_value(raw: Option<&Json>, server_ref: &str) -> Result<Option<bool>, ConfigError> {
    match raw {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(other) => Err(ConfigError::DcrBridgeType {
            server_ref: server_ref.to_owned(),
            raw: python_repr(other),
        }),
    }
}

pub fn config_per_server_oauth_discovery(
    config: &McpServerConfig,
    server_ref: &str,
    auth_type: Option<MCPAuth>,
    oauth2_flow: Option<OAuth2Flow>,
    delegate_auth_to_upstream: bool,
) -> Result<bool, ConfigError> {
    let enabled = match config.per_server_oauth_discovery.as_ref() {
        None => false,
        Some(Json::Bool(flag)) => *flag,
        Some(other) => {
            return Err(ConfigError::PerServerDiscoveryType {
                server_ref: server_ref.to_owned(),
                raw: python_repr(other),
            });
        }
    };
    if enabled
        && !is_per_server_oauth_discovery_eligible(
            auth_type,
            oauth2_flow,
            delegate_auth_to_upstream,
        )
    {
        return Err(ConfigError::PerServerDiscoveryEligible {
            server_ref: server_ref.to_owned(),
        });
    }
    Ok(enabled)
}
