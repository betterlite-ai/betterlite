use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::ModelLimits;
use betterlite_auth::auth_limit_inputs::metadata_limits::get_team_mcp_rpm_limit;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::descriptor::rate_limit_descriptor;
use crate::limiter_records::RateLimitDescriptor;

mod tests;

pub type SourceTeamLimits = BTreeMap<String, ModelLimits>;

// LiteLLM's _add_mcp_per_team_rate_limit_descriptor: the key's team and every source team charge their own bucket
pub fn add_mcp_per_team_descriptor(
    auth: &UserAPIKeyAuth,
    mcp_server_name: Option<&str>,
    source_team_rpm_limits: Option<&SourceTeamLimits>,
    window_size: i64,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(server) = mcp_server_name else {
        return;
    };
    if let Some(team_id) = account_limits(auth).team_id
        && let Some(team_limits) = get_team_mcp_rpm_limit(auth)
    {
        push_team_descriptor(out, team_id, &team_limits, server, window_size);
    }
    if let Some(source_teams) = source_team_rpm_limits {
        for (team_id, team_limits) in source_teams {
            push_team_descriptor(out, team_id, team_limits, server, window_size);
        }
    }
}

fn push_team_descriptor(
    out: &mut Vec<RateLimitDescriptor>,
    team_id: &str,
    team_limits: &ModelLimits,
    mcp_server_name: &str,
    window_size: i64,
) {
    let Some(server_rpm_limit) = team_limits.get(mcp_server_name) else {
        return;
    };
    out.push(rate_limit_descriptor(
        "mcp_per_team",
        format!("{team_id}:{mcp_server_name}"),
        Some(*server_rpm_limit),
        None,
        window_size,
    ));
}
