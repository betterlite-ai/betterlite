use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::Ceilings;
use super::error::{CeilingsError, CeilingsResult};
use super::live::decode;
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::tools::narrow_to_grants;

// the end-user level: the end user's own server and tool entitlements
pub trait EndUserCeilings: Sized {
    type Source: StdError + Send + Sync + 'static;
    fn end_user_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Vec<String>, CeilingsError<Self::Source>>> + Send;
    fn apply_end_user_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
}

impl<Reads: CeilingReads> EndUserCeilings for Ceilings<Reads> {
    type Source = Reads::Error;

    // _get_allowed_mcp_servers_for_end_user: this level adds no toolset servers, and a union fault reads as no restriction
    async fn end_user_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> CeilingsResult<Vec<String>, Reads> {
        let record = decode::<Reads::Error>(auth)?;
        if record.end_user_id.is_none() || !self.reads.has_database() {
            return Ok(Vec::new());
        }
        let loaded = self.reads.end_user_object_permission(auth).await?;
        let Some(permission) = loaded else {
            return Ok(Vec::new());
        };
        let fresh = self.reads.requires_fresh_policy(auth);
        let listed =
            servers::permission_servers(&self.reads, &permission, fresh, ToolsetTerm::Skipped);
        match listed.await {
            Ok(server_ids) => Ok(server_ids),
            Err(error) => {
                warn!("Failed to get allowed MCP servers for end user: {error}");
                Ok(Vec::new())
            }
        }
    }

    // _apply_end_user_tool_ceiling: an end user with nothing to read keeps every tool, and an end-user fault is returned
    async fn apply_end_user_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
    ) -> CeilingsResult<Option<Vec<String>>, Reads> {
        let record = decode::<Reads::Error>(auth)?;
        if record.end_user_id.is_none() || !self.reads.has_database() {
            return Ok(allowed);
        }
        let loaded = self.reads.end_user_object_permission(auth).await?;
        narrow_to_grants(self, server_id, auth, allowed, loaded).await
    }
}
