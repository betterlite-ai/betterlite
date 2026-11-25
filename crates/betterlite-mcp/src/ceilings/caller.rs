use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::debug;

use super::Ceilings;
use super::error::CeilingsResult;
use super::reads::CeilingReads;
use super::servers;
use super::user::apply_user_server_ceiling;
use super::{Capped, CeilingSource};

// apply_agent_caller_ceiling: the echoed team caps only when it grants servers, then the echoed user's own ceiling narrows what is left
pub(super) async fn apply_agent_caller_ceiling<Reads>(
    ceilings: &Ceilings<Reads>,
    allowed: Vec<String>,
    auth: Option<&UserAPIKeyAuth>,
) -> CeilingsResult<Capped, Reads>
where
    Reads: CeilingReads,
{
    let Some(source) = auth else {
        return Ok(Capped {
            servers: allowed,
            restricts: false,
        });
    };
    let Some(caller) = ceilings.reads.agent_caller_auth(source).await? else {
        return Ok(Capped {
            servers: allowed,
            restricts: false,
        });
    };
    let team_servers = ceilings.reads.team_servers(Some(&caller)).await?;
    let team_restricts = !team_servers.is_empty();
    let team_capped = if team_restricts {
        servers::capped(allowed, &team_servers)
    } else {
        allowed
    };
    let user_capped =
        apply_user_server_ceiling(ceilings, team_capped, Some(&caller), CeilingSource::Key).await?;
    debug!(
        "Applied agent caller ceiling. Final allowed servers: {:?}",
        user_capped.servers
    );
    Ok(Capped {
        restricts: team_restricts || user_capped.restricts,
        ..user_capped
    })
}
