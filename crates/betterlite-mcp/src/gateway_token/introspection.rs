mod tests;

use chrono::{DateTime, Utc};

use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::REFRESH_CLAIM_KEY_PREFIX;
use crate::gateway_dcr::oauth_error::OauthErrorCode::TemporarilyUnavailable;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode::ServerError};
use crate::gateway_dcr::ports::{ReloadUser, ReloadUserFailure};
use crate::gateway_dcr::single_use::{
    CLAIM_UNAVAILABLE_DESCRIPTION, ClaimBackend, PeekOutcome, SingleUseGuard,
};
use crate::session_tokens::{SESSION_ISSUER, SessionPrefix as _, open_session_token};
use crate::types::{OpenedSessionToken, SessionSigningKeys, SessionTokenKind};

use super::support::reply::{IntrospectionBody, TokenBody, TokenReply, json_reply};
use super::support::{BEARER_TOKEN_TYPE, SigningSetup, no_master_key_error, reload_failure_error};

#[must_use]
pub async fn introspect_gateway_token<R, B>(
    token: &str,
    setup: &SigningSetup,
    reload_user: &R,
    guard: &SingleUseGuard<B>,
    now: DateTime<Utc>,
) -> TokenReply
where
    R: ReloadUser,
    B: ClaimBackend,
{
    let keys = match introspect_keys(setup) {
        Ok(keys) => keys,
        Err(reply) => return reply,
    };
    let kind = if token.is_session_token() {
        SessionTokenKind::Session
    } else if token.is_session_refresh_token() {
        SessionTokenKind::Refresh
    } else {
        return inactive_reply();
    };
    let Ok(opened) = open_session_token(token, kind, keys, now) else {
        return inactive_reply();
    };
    match live_token(&opened, reload_user, guard).await {
        Ok(()) => active_reply(opened),
        Err(reply) => reply,
    }
}

fn introspect_keys(setup: &SigningSetup) -> Result<&SessionSigningKeys, TokenReply> {
    match setup {
        SigningSetup::Ready(keys) => Ok(keys),
        SigningSetup::NoMasterKey => Err(TokenReply::Error(no_master_key_error())),
        // docs/deviations/B4676.yaml: introspection names the signing defect itself
        SigningSetup::Invalid(error) => Err(TokenReply::Error(OauthError {
            status: 500,
            error: ServerError,
            description: error.to_string(),
        })),
    }
}

async fn live_token<R, B>(
    opened: &OpenedSessionToken,
    reload_user: &R,
    guard: &SingleUseGuard<B>,
) -> Result<(), TokenReply>
where
    R: ReloadUser,
    B: ClaimBackend,
{
    if opened.kind == SessionTokenKind::Refresh {
        let claim_key = format!("{REFRESH_CLAIM_KEY_PREFIX}{}", opened.jti);
        match guard.peek(&claim_key).await {
            PeekOutcome::Unclaimed => {}
            PeekOutcome::Claimed => return Err(inactive_reply()),
            PeekOutcome::Unavailable => {
                return Err(TokenReply::Error(oauth_error(
                    503,
                    TemporarilyUnavailable,
                    CLAIM_UNAVAILABLE_DESCRIPTION,
                )));
            }
        }
    }
    if let Some(failure) = reload_user.reload_user(&opened.principal.user_id).await {
        match failure {
            ReloadUserFailure::Unavailable | ReloadUserFailure::Faulted => {
                return Err(TokenReply::Error(reload_failure_error(failure)));
            }
            ReloadUserFailure::Unresolvable | ReloadUserFailure::NoActiveKey => {
                return Err(inactive_reply());
            }
        }
    }
    Ok(())
}

fn inactive_reply() -> TokenReply {
    json_reply(
        200,
        TokenBody::Introspection(IntrospectionBody {
            active: false,
            iss: None,
            sub: None,
            client_id: None,
            jti: None,
            iat: None,
            exp: None,
            kind: None,
            token_type: None,
            team_id: None,
            resource_server_id: None,
            audience: None,
        }),
    )
}

fn active_reply(opened: OpenedSessionToken) -> TokenReply {
    let OpenedSessionToken {
        principal,
        jti,
        kind,
        iat,
        exp,
    } = opened;
    let token_type = match kind {
        SessionTokenKind::Session => Some(BEARER_TOKEN_TYPE),
        SessionTokenKind::Refresh => None,
    };
    json_reply(
        200,
        TokenBody::Introspection(IntrospectionBody {
            active: true,
            iss: Some(SESSION_ISSUER),
            sub: Some(principal.user_id),
            client_id: Some(principal.client_id),
            jti: Some(jti),
            iat: Some(iat),
            exp: Some(exp),
            kind: Some(kind),
            token_type,
            team_id: principal.team_id,
            resource_server_id: principal.resource_server_id,
            audience: principal.audience,
        }),
    )
}
