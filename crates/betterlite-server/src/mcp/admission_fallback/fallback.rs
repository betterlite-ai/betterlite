mod tests;

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::slice;

use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_mcp::request_headers::path_targets::cold_start_server_names;
use thiserror::Error;
use tracing::debug;

use super::challenge::{
    ChallengeContext, ChallengeResponse, McpAuthHeaders, gateway_dcr_challenge,
    has_client_supplied_mcp_auth, is_dcr_challenge_scope,
};
use super::targets::{TargetQuery, is_legacy_delegate_cold_start, is_passthrough_cold_start};

#[derive(Clone, Copy)]
pub struct FallbackFlags {
    pub error_is_401: bool,
    pub bearer_presented: bool,
}

#[derive(Debug, Error)]
pub enum FallbackError<E>
where
    E: StdError + 'static,
{
    #[error("the MCP admission error became a gateway sign-in challenge")]
    Challenge(ChallengeResponse),
    #[error("the MCP admission error is returned unchanged")]
    Original(#[source] E),
    #[error("the anonymous MCP principal could not be built")]
    DefaultKey(#[source] AuthError),
}

pub fn admission_failure_fallback<E>(
    query: &TargetQuery<'_>,
    ctx: &ChallengeContext<'_>,
    auth: &McpAuthHeaders<'_>,
    flags: FallbackFlags,
    error: E,
) -> Result<UserAPIKeyAuth, FallbackError<E>>
where
    E: StdError + 'static,
{
    let from_path = cold_start_server_names(query.route, query.header_servers);
    if let Some(name) = from_path
        && !has_client_supplied_mcp_auth(auth.mcp_auth_header, auth.server_headers)
        && flags.error_is_401
    {
        let names = slice::from_ref(&name);
        let passthrough = is_passthrough_cold_start(query.registry, names, query.client_ip);
        let legacy = !flags.bearer_presented
            && is_legacy_delegate_cold_start(query.registry, names, query.client_ip);
        if passthrough || legacy {
            debug!("MCP pass-through cold start: deferring admission to route 401 emitter");
            return UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
                .map_err(FallbackError::DefaultKey);
        }
    }
    if is_dcr_challenge_scope(query, auth, flags.error_is_401) {
        let challenge = gateway_dcr_challenge(ctx, query, flags.bearer_presented);
        return Err(FallbackError::Challenge(challenge));
    }
    Err(FallbackError::Original(error))
}
