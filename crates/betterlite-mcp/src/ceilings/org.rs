use std::error::Error as StdError;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::{debug, warn};

use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::{Capped, CeilingServers, CeilingSource, Ceilings};
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::auth::object_permission_hydration::record::McpAuth;

// _get_allowed_mcp_servers_for_org: an empty union is Servers([]), never "no restriction"
pub(super) async fn org_servers<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
) -> CeilingsResult<CeilingServers, Reads>
where
    Reads: CeilingReads,
{
    let fresh = ceilings.reads.requires_fresh_policy(auth);
    let permission = match ceilings.reads.org_object_permission(auth).await {
        Ok(Some(permission)) => permission,
        Ok(None) => return Ok(CeilingServers::NoRestriction),
        Err(error) => return org_fault(error),
    };
    org_server_list(ceilings, &permission, fresh).await
}

// the org's own list over one permission row: the union fault is an unresolved ceiling, never a missing one
async fn org_server_list<Reads>(
    ceilings: &Ceilings<Reads>,
    permission: &ObjectPermission,
    fresh: bool,
) -> CeilingsResult<CeilingServers, Reads>
where
    Reads: CeilingReads,
{
    let listed =
        servers::permission_servers(&ceilings.reads, permission, fresh, ToolsetTerm::Counted);
    match listed.await {
        Ok(server_ids) => Ok(CeilingServers::Servers(server_ids)),
        Err(error) => org_fault(error),
    }
}

// a level that doesn't restrict answers the list it was handed
fn unchanged(allowed: Vec<String>) -> Capped {
    Capped {
        servers: allowed,
        restricts: false,
    }
}

// _apply_primary_org_ceiling: an org list caps a lower restriction or a keyless source and replaces a key's list otherwise
pub(super) async fn apply_primary_org_ceiling<Reads>(
    ceilings: &Ceilings<Reads>,
    allowed: Vec<String>,
    auth: Option<&UserAPIKeyAuth>,
    has_lower_level_restrictions: bool,
    source_kind: CeilingSource,
) -> CeilingsResult<Capped, Reads>
where
    Reads: CeilingReads,
{
    let Some(record) = auth.and_then(|caller| McpAuth::from_auth(caller).ok()) else {
        return Ok(unchanged(allowed));
    };
    let Some(org_id) = record.org_id else {
        return Ok(unchanged(allowed));
    };
    Ok(match ceilings.org_servers(auth).await? {
        CeilingServers::NoRestriction => unchanged(allowed),
        CeilingServers::Unresolved => Capped {
            servers: unresolved(allowed, source_kind, &org_id),
            restricts: false,
        },
        CeilingServers::Servers(org_servers) if org_servers.is_empty() => unchanged(allowed),
        CeilingServers::Servers(org_servers) => {
            let intersects =
                has_lower_level_restrictions || matches!(source_kind, CeilingSource::Keyless);
            let servers = if intersects {
                servers::capped(allowed, &org_servers)
            } else {
                org_servers
            };
            debug!("Applied org ceiling filter. Final allowed servers: {servers:?}");
            Capped {
                servers,
                restricts: true,
            }
        }
    })
}

// an unloadable named row or toolset denies every caller shape, while any other fault leaves the ceiling unresolved
fn org_fault<Source>(
    error: CeilingReadError<Source>,
) -> Result<CeilingServers, CeilingsError<Source>>
where
    Source: StdError + 'static,
{
    match error {
        CeilingReadError::Unloadable(unloadable) => Err(CeilingsError::Unloadable(unloadable)),
        CeilingReadError::Failed(message) => {
            warn!("Failed to get allowed MCP servers for org: {message}");
            Ok(CeilingServers::Unresolved)
        }
    }
}

// an unresolved org ceiling empties a keyless source and leaves a key's list alone, and neither is a restriction
fn unresolved(allowed: Vec<String>, source_kind: CeilingSource, org_id: &str) -> Vec<String> {
    match source_kind {
        CeilingSource::Key => {
            warn!("MCP org ceiling unresolved for org {org_id:?}; leaving uncapped (key auth)");
            allowed
        }
        CeilingSource::Keyless => {
            warn!(
                "MCP org ceiling unresolved for org {org_id:?}; denying (keyless admitted subject)"
            );
            Vec::new()
        }
    }
}
