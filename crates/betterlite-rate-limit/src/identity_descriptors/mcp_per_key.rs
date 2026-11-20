use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::metadata_limits::get_key_mcp_rpm_limit;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::descriptor::rate_limit_descriptor;
use crate::limiter_records::RateLimitDescriptor;

mod tests;

// LiteLLM's _add_mcp_per_key_rate_limit_descriptor: MCP calls spend no tokens, so the bucket is RPM-only
pub fn add_mcp_per_key_descriptor(
    auth: &UserAPIKeyAuth,
    mcp_server_name: Option<&str>,
    window_size: i64,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(server) = mcp_server_name else {
        return;
    };
    let Some(api_key) = account_limits(auth).hashed_api_key else {
        return;
    };
    let Some(mcp_rpm_limit) = get_key_mcp_rpm_limit(auth) else {
        return;
    };
    let Some(server_rpm_limit) = mcp_rpm_limit.get(server) else {
        return;
    };
    out.push(rate_limit_descriptor(
        "mcp_per_key",
        format!("{api_key}:{server}"),
        Some(*server_rpm_limit),
        None,
        window_size,
    ));
}
