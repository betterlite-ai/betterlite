use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use tracing::{debug, warn};

use super::Ceilings;
use super::error::{CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::{Capped, CeilingLevels, CeilingServers, CeilingSource};
use crate::auth::object_permission_hydration::record::McpAuth;

// _get_allowed_mcp_servers_for_user: every fault is an unresolved ceiling, never "no restriction"
pub(super) async fn user_servers<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
) -> CeilingServers
where
    Reads: CeilingReads,
{
    let permission = match ceilings.reads.user_object_permission(auth).await {
        Ok(Some(permission)) => permission,
        Ok(None) => return CeilingServers::NoRestriction,
        Err(error) => {
            warn!("Failed to get allowed MCP servers for user: {error}");
            return CeilingServers::Unresolved;
        }
    };
    let fresh = ceilings.reads.requires_fresh_policy(auth);
    match servers::permission_servers(&ceilings.reads, &permission, fresh, ToolsetTerm::Counted)
        .await
    {
        Ok(server_ids) => CeilingServers::Servers(server_ids),
        Err(error) => {
            warn!("Failed to get allowed MCP servers for user: {error}");
            CeilingServers::Unresolved
        }
    }
}

// _apply_user_server_ceiling: a keyless source reads nothing, and an unresolved human entitlement raises
pub(super) async fn apply_user_server_ceiling<Reads>(
    ceilings: &Ceilings<Reads>,
    allowed: Vec<String>,
    auth: Option<&UserAPIKeyAuth>,
    source_kind: CeilingSource,
) -> CeilingsResult<Capped, Reads>
where
    Reads: CeilingReads,
{
    if matches!(source_kind, CeilingSource::Keyless) {
        return Ok(Capped {
            servers: allowed,
            restricts: false,
        });
    }
    let entitled = match ceilings.user_servers(auth).await {
        CeilingServers::NoRestriction => Vec::new(),
        CeilingServers::Unresolved => {
            return Err(CeilingsError::UserCeilingUnresolvable {
                user_id: auth
                    .and_then(|caller| McpAuth::from_auth(caller).ok())
                    .and_then(|record| record.user_id),
            });
        }
        CeilingServers::Servers(entitled) => entitled,
    };
    if entitled.is_empty() {
        return Ok(Capped {
            servers: allowed,
            restricts: false,
        });
    }
    let capped = Capped {
        servers: servers::capped(allowed, &entitled),
        restricts: true,
    };
    debug!(
        "Applied user ceiling filter. Final allowed servers: {:?}",
        capped.servers
    );
    Ok(capped)
}

// _user_places_mcp_ceiling: an unresolved entitlement bounds the human too, since skipping the resolver on a fault would widen access
pub(super) async fn user_places_ceiling<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
) -> bool
where
    Reads: CeilingReads,
{
    match ceilings.user_servers(auth).await {
        CeilingServers::NoRestriction => false,
        CeilingServers::Unresolved => true,
        CeilingServers::Servers(servers) => !servers.is_empty(),
    }
}

// admin_view_unscoped: a credential's own explicit server scope disqualifies, while an admitted subject's row binds through the user ceiling
pub(super) async fn admin_view_unscoped<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
) -> bool
where
    Reads: CeilingReads,
{
    let Some(caller) = auth else {
        return false;
    };
    let marks = ceilings.reads.admission_marks(caller);
    if marks.explicit_grants_only || !user_api_key_has_admin_view(caller) {
        return false;
    }
    let credential_scoped = !marks.admitted_user_subject
        && McpAuth::from_auth(caller)
            .ok()
            .and_then(|record| record.object_permission)
            .is_some_and(|permission| permission.mcp_servers.0.is_some());
    if credential_scoped {
        return false;
    }
    !user_places_ceiling(ceilings, Some(caller)).await
}
