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
use crate::registry::servers::ServerRegistry;
use crate::types::SessionPrincipal;

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

pub(super) fn resource_conflicts_with_scope(
    base_url: &str,
    registry: &ServerRegistry,
    resource: Option<&str>,
    sealed_resource_server_id: Option<&str>,
) -> bool {
    let (Some(resource), Some(sealed)) = (resource, sealed_resource_server_id) else {
        return false;
    };
    match resolve_scoped_resource_server(registry, base_url, Some(resource)) {
        Some(server) => server.fields.server_id.as_str() != sealed,
        None => true,
    }
}
