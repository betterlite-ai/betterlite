mod tests;

use betterlite_db::stored_cipher::DbCipher;
use chrono::{DateTime, Utc};

use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::{CLAIM_TTL_BUFFER_SECONDS, REFRESH_CLAIM_KEY_PREFIX};
use crate::gateway_dcr::models::is_gateway_dcr_client_id;
use crate::gateway_dcr::oauth_error::OauthErrorCode::TemporarilyUnavailable;
use crate::gateway_dcr::sealed::open_gateway_dcr_client;
use crate::gateway_dcr::single_use::{
    CLAIM_UNAVAILABLE_DESCRIPTION, ClaimBackend, ClaimOutcome, SingleUseGuard,
};
use crate::session_credentials::{SessionRefreshResult, open_session_refresh_bearer};
use crate::session_tokens::SESSION_REFRESH_TTL_SECONDS;

use super::support::reply::{RevokedBody, TokenBody, TokenReply, json_reply};
use super::support::{SigningSetup, signing_refusal, unknown_client_error};

pub struct RevokeRequest<'request> {
    pub token: &'request str,
    pub client_id: Option<&'request str>,
}

#[must_use]
pub async fn revoke_refresh_token<B>(
    request: &RevokeRequest<'_>,
    setup: &SigningSetup,
    guard: &SingleUseGuard<B>,
    cipher: &DbCipher,
    now: DateTime<Utc>,
) -> TokenReply
where
    B: ClaimBackend,
{
    let client_id = request.client_id.unwrap_or_default();
    if !is_gateway_dcr_client_id(request.client_id)
        || open_gateway_dcr_client(client_id, cipher).is_none()
    {
        return TokenReply::Error(unknown_client_error());
    }
    let keys = match signing_refusal(setup) {
        Ok(keys) => keys,
        Err(refusal) => return TokenReply::Error(refusal),
    };
    let opened = open_session_refresh_bearer(request.token, keys, now, client_id);
    if let SessionRefreshResult::SessionRefreshOpened { jti, .. } = opened {
        let claim_key = format!("{REFRESH_CLAIM_KEY_PREFIX}{jti}");
        let claim_ttl = SESSION_REFRESH_TTL_SECONDS.saturating_add(CLAIM_TTL_BUFFER_SECONDS);
        if let ClaimOutcome::Unavailable = guard.claim(&claim_key, claim_ttl).await {
            return TokenReply::Error(oauth_error(
                503,
                TemporarilyUnavailable,
                CLAIM_UNAVAILABLE_DESCRIPTION,
            ));
        }
    }
    json_reply(200, TokenBody::Revoked(RevokedBody::default()))
}
