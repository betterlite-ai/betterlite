use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::{debug, warn};

use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::live::decode;
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::{Capped, CeilingServers, CeilingSource, Ceilings};

// the org level: the permission's own server list and the step that applies it to a lower list
pub trait OrgCeilings: Sized {
    type Source: StdError + Send + Sync + 'static;
    fn org_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<CeilingServers, CeilingsError<Self::Source>>> + Send;
    fn apply_primary_org_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
        has_lower_level_restrictions: bool,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Capped, CeilingsError<Self::Source>>> + Send;
}

impl<Reads: CeilingReads> OrgCeilings for Ceilings<Reads> {
    type Source = Reads::Error;

    // _get_allowed_mcp_servers_for_org: an empty union is Servers([]), never "no restriction"
    async fn org_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> CeilingsResult<CeilingServers, Reads> {
        let fresh = self.reads.requires_fresh_policy(auth);
        let permission = match self.reads.org_object_permission(auth).await {
            Ok(Some(permission)) => permission,
            Ok(None) => return Ok(CeilingServers::NoRestriction),
            Err(error) => return org_fault(error),
        };
        // the union fault is an unresolved ceiling, never a missing one
        let listed =
            servers::permission_servers(&self.reads, &permission, fresh, ToolsetTerm::Counted);
        match listed.await {
            Ok(server_ids) => Ok(CeilingServers::Servers(server_ids)),
            Err(error) => org_fault(error),
        }
    }

    // _apply_primary_org_ceiling: an org list caps a lower restriction or a keyless source and replaces a key's list otherwise
    async fn apply_primary_org_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
        has_lower_level_restrictions: bool,
        source_kind: CeilingSource,
    ) -> CeilingsResult<Capped, Reads> {
        let record = decode::<Reads::Error>(auth)?;
        let Some(org_id) = record.org_id else {
            return Ok(Capped {
                servers: allowed,
                restricts: false,
            });
        };
        Ok(match self.org_servers(auth).await? {
            CeilingServers::NoRestriction => Capped {
                servers: allowed,
                restricts: false,
            },
            CeilingServers::Unresolved => Capped {
                servers: unresolved(allowed, source_kind, &org_id),
                restricts: false,
            },
            CeilingServers::Servers(org_servers) if org_servers.is_empty() => Capped {
                servers: allowed,
                restricts: false,
            },
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
