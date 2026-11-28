mod codes;
mod tests;

use betterlite_db::stored_cipher::DbCipher;
use chrono::{DateTime, Utc};

use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::{
    ACCESS_TOKEN_TOKEN_TYPE, CLAIM_TTL_BUFFER_SECONDS, CODE_CLAIM_KEY_PREFIX,
    REFRESH_CLAIM_KEY_PREFIX, SUBJECT_TOKEN_TYPES, TOKEN_EXCHANGE_GRANT_TYPE,
};
use crate::gateway_dcr::models::is_gateway_dcr_client_id;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{
    InvalidGrant, InvalidRequest, InvalidTarget, UnsupportedGrantType,
};
use crate::gateway_dcr::ports::{MintProxyCredential, ReloadUser};
use crate::gateway_dcr::sealed::open_gateway_dcr_client;
use crate::gateway_dcr::single_use::{ClaimBackend, SingleUseGuard};
use crate::gateway_dcr::subject_token::ExchangeSubjectToken;
use crate::registry::servers::ServerRegistry;
use crate::session_credentials::{SessionRefreshResult, open_session_refresh_bearer};
use crate::session_tokens::SESSION_REFRESH_TTL_SECONDS;

use self::codes::{
    check_code_binding, code_principal, code_proof, present, resource_conflicts_with_scope,
    sealed_code,
};
use super::issuer::GrantIssuer;
use super::support::reply::TokenReply;
use super::support::{
    SessionEntropy, SigningSetup, pkce_verifier_matches, signing_refusal, unknown_client_error,
};

pub struct TokenForm<'form> {
    pub grant_type: Option<&'form str>,
    pub code: Option<&'form str>,
    pub redirect_uri: Option<&'form str>,
    pub client_id: Option<&'form str>,
    pub code_verifier: Option<&'form str>,
    pub refresh_token: Option<&'form str>,
    pub resource: Option<&'form str>,
    pub subject_token: Option<&'form str>,
    pub subject_token_type: Option<&'form str>,
    pub requested_token_type: Option<&'form str>,
}

pub struct TokenPorts<'ports, R, M, B, S, Context> {
    pub registry: &'ports ServerRegistry,
    pub cipher: &'ports DbCipher,
    pub base_url: &'ports str,
    pub entropy: &'ports dyn SessionEntropy,
    pub reload_user: &'ports R,
    pub mint_proxy_credential: &'ports M,
    pub guard: &'ports SingleUseGuard<B>,
    pub exchange_subject_token: &'ports S,
    pub exchange_context: &'ports Context,
}

struct TokenCall<'call, R, M, B, S, Context> {
    issuer: &'call GrantIssuer<'call, R, M, B, S, Context>,
    ports: &'call TokenPorts<'call, R, M, B, S, Context>,
}

impl<R, M, B, S, Context> TokenCall<'_, R, M, B, S, Context>
where
    R: ReloadUser,
    M: MintProxyCredential,
    B: ClaimBackend,
    S: ExchangeSubjectToken<Context>,
{
    async fn authorization_code(&self, form: &TokenForm<'_>) -> TokenReply {
        let proof = match code_proof(form) {
            Ok(proof) => proof,
            Err(refusal) => return TokenReply::Error(refusal),
        };
        let auth_code = match sealed_code(&proof, self.ports.cipher) {
            Ok(auth_code) => auth_code,
            Err(refusal) => return TokenReply::Error(refusal),
        };
        if let Err(refusal) = check_code_binding(&auth_code, &proof, form, self.issuer.now) {
            return TokenReply::Error(refusal);
        }
        let sealed_server = auth_code.resource_server_id.as_deref();
        if resource_conflicts_with_scope(
            self.issuer.base_url,
            self.ports.registry,
            form.resource,
            sealed_server,
        ) {
            return TokenReply::Error(oauth_error(
                400,
                InvalidTarget,
                "resource does not match the scope this code was issued for",
            ));
        }
        if !pkce_verifier_matches(proof.code_verifier, &auth_code.code_challenge) {
            return TokenReply::Error(oauth_error(400, InvalidGrant, "PKCE verification failed"));
        }
        let claim_key = format!("{CODE_CLAIM_KEY_PREFIX}{}", auth_code.jti);
        let claim_ttl = auth_code
            .exp
            .saturating_sub(self.issuer.now.timestamp())
            .saturating_add(CLAIM_TTL_BUFFER_SECONDS);
        self.issuer
            .issue(
                code_principal(auth_code, form),
                &claim_key,
                claim_ttl,
                "the authorization code was already used",
            )
            .await
    }

    async fn refresh_token(&self, form: &TokenForm<'_>) -> TokenReply {
        let Some(refresh_token) = present(form.refresh_token) else {
            return TokenReply::Error(oauth_error(
                400,
                InvalidRequest,
                "refresh_token is required",
            ));
        };
        let client_id = form.client_id.unwrap_or_default();
        let opened = open_session_refresh_bearer(
            refresh_token,
            self.issuer.keys,
            self.issuer.now,
            client_id,
        );
        let SessionRefreshResult::SessionRefreshOpened { principal, jti } = opened else {
            return TokenReply::Error(oauth_error(
                400,
                InvalidGrant,
                "the refresh token is invalid for this client",
            ));
        };
        let sealed_server = principal.resource_server_id.as_deref();
        if resource_conflicts_with_scope(
            self.issuer.base_url,
            self.ports.registry,
            form.resource,
            sealed_server,
        ) {
            return TokenReply::Error(oauth_error(
                400,
                InvalidTarget,
                "resource does not match the scope this token was issued for",
            ));
        }
        let claim_key = format!("{REFRESH_CLAIM_KEY_PREFIX}{jti}");
        let claim_ttl = SESSION_REFRESH_TTL_SECONDS.saturating_add(CLAIM_TTL_BUFFER_SECONDS);
        self.issuer
            .issue(
                principal,
                &claim_key,
                claim_ttl,
                "the refresh token was already used",
            )
            .await
    }

    async fn token_exchange(&self, form: &TokenForm<'_>) -> TokenReply {
        let client_id = form.client_id.unwrap_or_default();
        if !is_gateway_dcr_client_id(form.client_id)
            || open_gateway_dcr_client(client_id, self.ports.cipher).is_none()
        {
            return TokenReply::Error(unknown_client_error());
        }
        let (Some(subject_token), Some(subject_token_type)) = (
            present(form.subject_token),
            present(form.subject_token_type),
        ) else {
            return TokenReply::Error(oauth_error(
                400,
                InvalidRequest,
                "subject_token and subject_token_type are required",
            ));
        };
        if !SUBJECT_TOKEN_TYPES.contains(&subject_token_type) {
            let mut accepted = SUBJECT_TOKEN_TYPES;
            accepted.sort_unstable();
            let listed = accepted.join(", ");
            let description = format!("subject_token_type must be one of {listed}");
            return TokenReply::Error(oauth_error(400, InvalidRequest, &description));
        }
        let wrong_requested = form
            .requested_token_type
            .is_some_and(|requested| requested != ACCESS_TOKEN_TOKEN_TYPE);
        if wrong_requested {
            let description = format!("requested_token_type must be {ACCESS_TOKEN_TOKEN_TYPE}");
            return TokenReply::Error(oauth_error(400, InvalidRequest, &description));
        }
        self.issuer.exchange(subject_token, client_id).await
    }
}

#[must_use]
pub async fn aggregate_token<R, M, B, S, Context>(
    setup: &SigningSetup,
    form: &TokenForm<'_>,
    ports: &TokenPorts<'_, R, M, B, S, Context>,
    now: DateTime<Utc>,
) -> TokenReply
where
    R: ReloadUser,
    M: MintProxyCredential,
    B: ClaimBackend,
    S: ExchangeSubjectToken<Context>,
{
    let issuer = match ports.issuer(setup, form, now) {
        Ok(issuer) => issuer,
        Err(refusal) => return refusal,
    };
    let call = TokenCall {
        issuer: &issuer,
        ports,
    };
    match form.grant_type {
        Some("authorization_code") => call.authorization_code(form).await,
        Some("refresh_token") => call.refresh_token(form).await,
        Some(grant) if grant == TOKEN_EXCHANGE_GRANT_TYPE => call.token_exchange(form).await,
        None | Some(_) => {
            let description = format!(
                "grant_type must be authorization_code, refresh_token, or {TOKEN_EXCHANGE_GRANT_TYPE}"
            );
            TokenReply::Error(oauth_error(400, UnsupportedGrantType, &description))
        }
    }
}

impl<R, M, B, S, Context> TokenPorts<'_, R, M, B, S, Context>
where
    R: ReloadUser,
    M: MintProxyCredential,
    B: ClaimBackend,
    S: ExchangeSubjectToken<Context>,
{
    fn issuer<'request>(
        &'request self,
        setup: &'request SigningSetup,
        form: &'request TokenForm<'request>,
        now: DateTime<Utc>,
    ) -> Result<GrantIssuer<'request, R, M, B, S, Context>, TokenReply> {
        let keys = match signing_refusal(setup) {
            Ok(keys) => keys,
            Err(refusal) => return Err(TokenReply::Error(refusal)),
        };
        Ok(GrantIssuer {
            keys,
            resource: form.resource,
            base_url: self.base_url,
            now,
            entropy: self.entropy,
            reload_user: self.reload_user,
            mint_proxy_credential: self.mint_proxy_credential,
            exchange_subject_token: self.exchange_subject_token,
            exchange_context: self.exchange_context,
            guard: self.guard,
        })
    }
}
