use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::{debug, warn};

use super::Ceilings;
use super::error::{CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers;
use super::tools::{narrowed, team_tool_grants};
use super::user::UserCeilings as _;
use super::{Capped, CeilingSource};

// the agent caller level: the echoed team's servers and tools, then the echoed user's own ceilings
pub trait AgentCallerCeilings: Sized {
    type Source: StdError + Send + Sync + 'static;
    fn apply_agent_caller_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Capped, CeilingsError<Self::Source>>> + Send;
    // ast-grep-ignore: no-long-names - apply_agent_caller_tool_ceiling narrows the echoed team's then the echoed user's grants
    fn apply_agent_caller_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
}

impl<Reads: CeilingReads> AgentCallerCeilings for Ceilings<Reads> {
    type Source = Reads::Error;

    // apply_agent_caller_ceiling: the echoed team caps only when it grants servers, then the echoed user's own ceiling narrows what is left
    async fn apply_agent_caller_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
    ) -> CeilingsResult<Capped, Reads> {
        let Some(source) = auth else {
            return Ok(Capped {
                servers: allowed,
                restricts: false,
            });
        };
        let Some(caller) = self.reads.agent_caller_auth(source).await? else {
            return Ok(Capped {
                servers: allowed,
                restricts: false,
            });
        };
        let team_servers = self.reads.team_servers(Some(&caller)).await?;
        let team_restricts = !team_servers.is_empty();
        let team_capped = if team_restricts {
            servers::capped(allowed, &team_servers)
        } else {
            allowed
        };
        let user_capped = self
            .apply_user_server_ceiling(team_capped, Some(&caller), CeilingSource::Key)
            .await?;
        debug!(
            "Applied agent caller ceiling. Final allowed servers: {:?}",
            user_capped.servers
        );
        Ok(Capped {
            restricts: team_restricts || user_capped.restricts,
            ..user_capped
        })
    }

    // ast-grep-ignore: no-long-names - a caller team that cannot be read denies every tool, then the echoed user's own tool ceiling
    async fn apply_agent_caller_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
    ) -> CeilingsResult<Option<Vec<String>>, Reads> {
        let Some(source) = auth else {
            return Ok(allowed);
        };
        let Some(caller) = self.reads.agent_caller_auth(source).await? else {
            return Ok(allowed);
        };
        let team = match team_tool_grants(self, &caller, server_id).await {
            Ok(team) => team,
            Err(error) => {
                warn!("MCP agent caller team tool ceiling unresolvable on {server_id:?}: {error}");
                return Ok(Some(Vec::new()));
            }
        };
        let team_capped = narrowed(allowed, team);
        self.apply_user_tool_ceiling(team_capped, server_id, Some(&caller), CeilingSource::Key)
            .await
    }
}
