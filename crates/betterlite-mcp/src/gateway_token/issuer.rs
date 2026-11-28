mod tests;

use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;

use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::scope::is_proxy_api_resource;
use crate::gateway_dcr::contracts::ACCESS_TOKEN_TOKEN_TYPE;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::ports::{MintProxyCredential, MintedProxyCredential, ReloadUser};
use crate::gateway_dcr::single_use::{ClaimBackend, SingleUseGuard, claim_refusal};
use crate::gateway_dcr::subject_token::ExchangeSubjectToken;
use crate::session_tokens::mint_session_token;
use crate::types::{SessionAudience, SessionPrincipal, SessionSigningKeys, SessionTokenKind};

use super::support::reply::{
    ProxyCredentialBody, SessionPairBody, TokenBody, TokenReply, json_reply,
};
use super::support::{
    BEARER_TOKEN_TYPE, SessionEntropy, failed_to_mint_error, mint_failure_error,
    reload_failure_error, subject_refusal_error,
};

pub struct GrantIssuer<'setup, R, M, B, S, Context> {
    pub keys: &'setup SessionSigningKeys,
    pub resource: Option<&'setup str>,
    pub base_url: &'setup str,
    pub now: DateTime<Utc>,
    pub entropy: &'setup dyn SessionEntropy,
    pub reload_user: &'setup R,
    pub mint_proxy_credential: &'setup M,
    pub exchange_subject_token: &'setup S,
    pub exchange_context: &'setup Context,
    pub guard: &'setup SingleUseGuard<B>,
}

struct Claim<'claim> {
    key: &'claim str,
    ttl_seconds: i64,
    replayed_message: &'claim str,
}

impl<R, M, B, S, Context> GrantIssuer<'_, R, M, B, S, Context>
where
    R: ReloadUser,
    M: MintProxyCredential,
    B: ClaimBackend,
    S: ExchangeSubjectToken<Context> + Sync,
    Context: Sync,
{
    #[must_use]
    pub async fn issue(
        &self,
        principal: SessionPrincipal,
        claim_key: &str,
        claim_ttl_seconds: i64,
        replayed_message: &str,
    ) -> TokenReply {
        let claim = Claim {
            key: claim_key,
            ttl_seconds: claim_ttl_seconds,
            replayed_message,
        };
        match principal.audience {
            None => self.issue_session_pair(&principal, &claim).await,
            Some(SessionAudience::ProxyApi) => {
                self.issue_proxy_credential(&principal, &claim).await
            }
        }
    }

    #[must_use]
    pub async fn exchange(&self, subject_token: &str, client_id: &str) -> TokenReply {
        if let Some(refusal) = self.proxy_api_target_refusal() {
            return TokenReply::Error(refusal);
        }
        let identity = match self
            .exchange_subject_token
            .exchange(subject_token, self.exchange_context)
            .await
        {
            Ok(identity) => identity,
            Err(refusal) => return TokenReply::Error(subject_refusal_error(&refusal)),
        };
        let principal = SessionPrincipal {
            user_id: identity.user_id,
            client_id: client_id.to_owned(),
            resource_server_id: None,
            audience: Some(SessionAudience::ProxyApi),
            team_id: identity.team_id,
        };
        let minted = match self
            .mint_proxy_credential
            .mint(&principal.user_id, principal.team_id.as_deref())
            .await
        {
            Ok(minted) => minted,
            Err(failure) => return TokenReply::Error(mint_failure_error(failure)),
        };
        self.proxy_credential_reply(&minted, &principal, Some(ACCESS_TOKEN_TOKEN_TYPE))
    }

    async fn issue_session_pair(
        &self,
        principal: &SessionPrincipal,
        claim: &Claim<'_>,
    ) -> TokenReply {
        if let Some(failure) = self.reload_user.reload_user(&principal.user_id).await {
            return TokenReply::Error(reload_failure_error(failure));
        }
        if let Some(refusal) = self.claim(claim).await {
            return TokenReply::Error(refusal);
        }
        let minted_access = mint_session_token(
            principal,
            SessionTokenKind::Session,
            self.keys,
            self.now,
            || self.entropy.jti(),
        );
        let minted_refresh = mint_session_token(
            principal,
            SessionTokenKind::Refresh,
            self.keys,
            self.now,
            || self.entropy.jti(),
        );
        let (Ok(access), Ok(refresh)) = (minted_access, minted_refresh) else {
            return TokenReply::Error(failed_to_mint_error());
        };
        let expires_in = access
            .expires_at
            .signed_duration_since(self.now)
            .num_seconds();
        json_reply(
            200,
            TokenBody::SessionPair(SessionPairBody {
                access_token: access.token.expose_secret().to_owned(),
                token_type: BEARER_TOKEN_TYPE,
                expires_in,
                refresh_token: refresh.token.expose_secret().to_owned(),
            }),
        )
    }

    async fn issue_proxy_credential(
        &self,
        principal: &SessionPrincipal,
        claim: &Claim<'_>,
    ) -> TokenReply {
        if let Some(refusal) = self.proxy_api_target_refusal() {
            return TokenReply::Error(refusal);
        }
        let minted = match self
            .mint_proxy_credential
            .mint(&principal.user_id, principal.team_id.as_deref())
            .await
        {
            Ok(minted) => minted,
            Err(failure) => return TokenReply::Error(mint_failure_error(failure)),
        };
        if let Some(refusal) = self.claim(claim).await {
            return TokenReply::Error(refusal);
        }
        self.proxy_credential_reply(&minted, principal, None)
    }

    fn proxy_credential_reply(
        &self,
        minted: &MintedProxyCredential,
        principal: &SessionPrincipal,
        issued_token_type: Option<&'static str>,
    ) -> TokenReply {
        // the rotated refresh token keeps the minted team so a renewal stays in the consented team
        let bound = SessionPrincipal {
            user_id: principal.user_id.clone(),
            client_id: principal.client_id.clone(),
            resource_server_id: principal.resource_server_id.clone(),
            audience: principal.audience,
            team_id: minted.team_id.clone(),
        };
        let refresh = match mint_session_token(
            &bound,
            SessionTokenKind::Refresh,
            self.keys,
            self.now,
            || self.entropy.jti(),
        ) {
            Ok(refresh) => refresh,
            Err(_error) => return TokenReply::Error(failed_to_mint_error()),
        };
        json_reply(
            200,
            TokenBody::ProxyCredential(ProxyCredentialBody {
                access_token: minted.key.expose_secret().to_owned(),
                token_type: BEARER_TOKEN_TYPE,
                expires_in: minted.expires_in.get(),
                refresh_token: refresh.token.expose_secret().to_owned(),
                user_id: minted.user_id.clone(),
                team_id: minted.team_id.clone(),
                issued_token_type,
            }),
        )
    }

    fn proxy_api_target_refusal(&self) -> Option<OauthError> {
        if self.resource.is_none() || is_proxy_api_resource(self.base_url, self.resource) {
            return None;
        }
        Some(oauth_error(
            400,
            OauthErrorCode::InvalidTarget,
            "resource does not match the proxy API this grant was issued for",
        ))
    }

    async fn claim(&self, claim: &Claim<'_>) -> Option<OauthError> {
        let outcome = self.guard.claim(claim.key, claim.ttl_seconds).await;
        claim_refusal(
            outcome,
            oauth_error(400, OauthErrorCode::InvalidGrant, claim.replayed_message),
        )
    }
}
