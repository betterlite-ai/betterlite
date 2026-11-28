use betterlite_db::stored_cipher::DbCipher;
use chrono::{DateTime, Utc};

use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::scope::resolve_scoped_resource_server;
use crate::gateway_dcr::contracts::{
    AUTH_CODE_DEBUG_KEY, GATEWAY_AUTH_CODE_PREFIX, MAX_CODE_VERIFIER_LENGTH,
    MIN_CODE_VERIFIER_LENGTH,
};
use crate::gateway_dcr::models::GatewayAuthCode;
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{
    InvalidGrant, InvalidRequest, InvalidTarget,
};
use crate::gateway_dcr::sealed::open_sealed;
use crate::gateway_token::support::pkce_verifier_matches;
use crate::registry::servers::ServerRegistry;
use crate::session_credentials::{SessionRefreshResult, open_session_refresh_bearer};
use crate::types::{SessionPrincipal, SessionSigningKeys};

use super::TokenForm;

pub(super) struct CodeProof<'proof> {
    pub(super) code: &'proof str,
    pub(super) redirect_uri: &'proof str,
    pub(super) code_verifier: &'proof str,
}

pub(super) const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        None | Some(_) => None,
    }
}

pub(super) fn code_proof<'form>(form: &TokenForm<'form>) -> Result<CodeProof<'form>, OauthError> {
    let Some(code) = present(form.code) else {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "code, redirect_uri, and code_verifier are required",
        ));
    };
    let Some(redirect_uri) = present(form.redirect_uri) else {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "code, redirect_uri, and code_verifier are required",
        ));
    };
    let Some(code_verifier) = present(form.code_verifier) else {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "code, redirect_uri, and code_verifier are required",
        ));
    };
    let length = code_verifier.chars().count();
    if !(MIN_CODE_VERIFIER_LENGTH..=MAX_CODE_VERIFIER_LENGTH).contains(&length) {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "code_verifier must be 43 to 128 characters (RFC 7636)",
        ));
    }
    Ok(CodeProof {
        code,
        redirect_uri,
        code_verifier,
    })
}

pub(super) fn sealed_code(
    proof: &CodeProof<'_>,
    cipher: &DbCipher,
) -> Result<GatewayAuthCode, OauthError> {
    let Some(auth_code) = open_sealed::<GatewayAuthCode>(
        proof.code,
        GATEWAY_AUTH_CODE_PREFIX,
        cipher,
        AUTH_CODE_DEBUG_KEY,
    ) else {
        return Err(oauth_error(
            400,
            InvalidGrant,
            "the authorization code is invalid",
        ));
    };
    Ok(auth_code)
}

pub(super) fn check_code_binding(
    auth_code: &GatewayAuthCode,
    proof: &CodeProof<'_>,
    form: &TokenForm<'_>,
    now: DateTime<Utc>,
) -> Result<(), OauthError> {
    if now.timestamp() >= auth_code.exp {
        return Err(oauth_error(
            400,
            InvalidGrant,
            "the authorization code has expired",
        ));
    }
    if form.client_id.unwrap_or_default() != auth_code.client_id.as_str()
        || proof.redirect_uri != auth_code.redirect_uri.as_str()
    {
        return Err(oauth_error(
            400,
            InvalidGrant,
            "the authorization code was issued to a different client",
        ));
    }
    Ok(())
}

pub(super) fn code_principal(auth_code: GatewayAuthCode, form: &TokenForm<'_>) -> SessionPrincipal {
    SessionPrincipal {
        user_id: auth_code.user_id,
        client_id: form.client_id.unwrap_or_default().to_owned(),
        resource_server_id: auth_code.resource_server_id,
        audience: auth_code.audience,
        team_id: auth_code.team_id,
    }
}

pub(super) struct RefreshProof {
    pub(super) principal: SessionPrincipal,
    pub(super) jti: String,
}

pub(super) fn open_refresh(
    form: &TokenForm<'_>,
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
) -> Result<RefreshProof, OauthError> {
    let Some(refresh_token) = present(form.refresh_token) else {
        return Err(oauth_error(
            400,
            InvalidRequest,
            "refresh_token is required",
        ));
    };
    let client_id = form.client_id.unwrap_or_default();
    let SessionRefreshResult::SessionRefreshOpened { principal, jti } =
        open_session_refresh_bearer(refresh_token, keys, now, client_id)
    else {
        return Err(oauth_error(
            400,
            InvalidGrant,
            "the refresh token is invalid for this client",
        ));
    };
    Ok(RefreshProof { principal, jti })
}

pub(super) fn check_code_grant(
    auth_code: &GatewayAuthCode,
    proof: &CodeProof<'_>,
    form: &TokenForm<'_>,
    base_url: &str,
    registry: &ServerRegistry,
) -> Result<(), OauthError> {
    let sealed_server = auth_code.resource_server_id.as_deref();
    if resource_conflicts_with_scope(base_url, registry, form.resource, sealed_server) {
        return Err(oauth_error(
            400,
            InvalidTarget,
            "resource does not match the scope this code was issued for",
        ));
    }
    if !pkce_verifier_matches(proof.code_verifier, &auth_code.code_challenge) {
        return Err(oauth_error(400, InvalidGrant, "PKCE verification failed"));
    }
    Ok(())
}

pub(super) fn check_token_scope(
    sealed_resource_server_id: Option<&str>,
    resource: Option<&str>,
    base_url: &str,
    registry: &ServerRegistry,
) -> Result<(), OauthError> {
    if resource_conflicts_with_scope(base_url, registry, resource, sealed_resource_server_id) {
        return Err(oauth_error(
            400,
            InvalidTarget,
            "resource does not match the scope this token was issued for",
        ));
    }
    Ok(())
}

fn resource_conflicts_with_scope(
    base_url: &str,
    registry: &ServerRegistry,
    resource: Option<&str>,
    sealed_resource_server_id: Option<&str>,
) -> bool {
    let (Some(requested), Some(sealed)) = (resource, sealed_resource_server_id) else {
        return false;
    };
    resolve_scoped_resource_server(registry, base_url, Some(requested))
        .is_none_or(|server| server.fields.server_id.as_str() != sealed)
}
