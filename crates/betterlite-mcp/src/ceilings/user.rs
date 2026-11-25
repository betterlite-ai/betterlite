use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use tracing::{debug, warn};

use super::Ceilings;
use super::error::{CeilingsError, CeilingsResult};
use super::live::decode;
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::tools::narrow_to_grants;
use super::{Capped, CeilingServers, CeilingSource};

// the human user level: the entitlement read, its server and tool ceilings and the two bounds that read it
pub trait UserCeilings: Sized {
    type Source: StdError + Send + Sync + 'static;
    fn user_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = CeilingServers> + Send;
    fn apply_user_server_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Capped, CeilingsError<Self::Source>>> + Send;
    fn apply_user_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    fn user_places_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = bool> + Send;
    fn admin_view_unscoped(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = bool> + Send;
}

impl<Reads: CeilingReads> UserCeilings for Ceilings<Reads> {
    type Source = Reads::Error;

    // _get_allowed_mcp_servers_for_user: every fault is an unresolved ceiling, never "no restriction"
    async fn user_servers(&self, auth: Option<&UserAPIKeyAuth>) -> CeilingServers {
        let permission = match self.reads.user_object_permission(auth).await {
            Ok(Some(permission)) => permission,
            Ok(None) => return CeilingServers::NoRestriction,
            Err(error) => {
                warn!("Failed to get allowed MCP servers for user: {error}");
                return CeilingServers::Unresolved;
            }
        };
        let fresh = self.reads.requires_fresh_policy(auth);
        let listed =
            servers::permission_servers(&self.reads, &permission, fresh, ToolsetTerm::Counted);
        match listed.await {
            Ok(server_ids) => CeilingServers::Servers(server_ids),
            Err(error) => {
                warn!("Failed to get allowed MCP servers for user: {error}");
                CeilingServers::Unresolved
            }
        }
    }

    // _apply_user_server_ceiling: a keyless source reads nothing, and an unresolved human entitlement raises
    async fn apply_user_server_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> CeilingsResult<Capped, Reads> {
        if matches!(source_kind, CeilingSource::Keyless) {
            return Ok(Capped {
                servers: allowed,
                restricts: false,
            });
        }
        let entitled = match self.user_servers(auth).await {
            CeilingServers::NoRestriction => Vec::new(),
            CeilingServers::Unresolved => {
                let record = decode::<Reads::Error>(auth)?;
                return Err(CeilingsError::UserCeilingUnresolvable {
                    user_id: record.user_id,
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

    // _apply_user_tool_ceiling: a keyless source keeps every tool, and a user fault denies every tool
    async fn apply_user_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> CeilingsResult<Option<Vec<String>>, Reads> {
        if matches!(source_kind, CeilingSource::Keyless) {
            return Ok(allowed);
        }
        let loaded = match self.reads.user_object_permission(auth).await {
            Ok(loaded) => loaded,
            Err(error) => {
                warn!(
                    "MCP user tool ceiling unresolvable, denying tools on {server_id:?}: {error}"
                );
                return Ok(Some(Vec::new()));
            }
        };
        narrow_to_grants(self, server_id, auth, allowed, loaded).await
    }

    // _user_places_mcp_ceiling: an unresolved entitlement bounds the human too, since skipping the resolver on a fault would widen access
    async fn user_places_ceiling(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        match self.user_servers(auth).await {
            CeilingServers::NoRestriction => false,
            CeilingServers::Unresolved => true,
            CeilingServers::Servers(servers) => !servers.is_empty(),
        }
    }

    // admin_view_unscoped: a credential's own explicit server scope disqualifies, while an admitted subject's row binds through the user ceiling
    async fn admin_view_unscoped(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        let Some(caller) = auth else {
            return false;
        };
        let marks = self.reads.admission_marks(caller);
        if marks.explicit_grants_only || !user_api_key_has_admin_view(caller) {
            return false;
        }
        let credential_scoped = if marks.admitted_user_subject {
            false
        } else {
            match decode::<Reads::Error>(Some(caller)) {
                Ok(record) => record
                    .object_permission
                    .is_some_and(|permission| permission.mcp_servers.0.is_some()),
                Err(error) => {
                    warn!(
                        "MCP admin view scope unreadable, treating it as credential scoped: {error}"
                    );
                    true
                }
            }
        };
        if credential_scoped {
            return false;
        }
        !self.user_places_ceiling(Some(caller)).await
    }
}
