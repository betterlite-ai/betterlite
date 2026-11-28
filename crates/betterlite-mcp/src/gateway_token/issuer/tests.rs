#![cfg(test)]

use std::future::{Future, ready};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU32, Ordering};

use aws_lc_rs::error::Unspecified;
use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;
use secrecy::SecretString;

use crate::gateway_authorize::tests::support::{BASE, NoBackend, guard};
use crate::gateway_dcr::contracts::ACCESS_TOKEN_TOKEN_TYPE;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::ports::{
    MintProxyCredential, MintedProxyCredential, ProxyCredentialMintFailure, ReloadUser,
    ReloadUserFailure,
};
use crate::gateway_dcr::single_use::{ClaimAuthority, SingleUseGuard};
use crate::gateway_dcr::subject_token::{
    ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal,
};
use crate::gateway_token::support::SessionEntropy;
use crate::gateway_token::support::reply::{
    ProxyCredentialBody, SessionPairBody, TokenBody, TokenReply,
};
use crate::session_tokens::{SESSION_REFRESH_TTL_SECONDS, SESSION_TTL_SECONDS, open_session_token};
use crate::types::{
    SessionAudience, SessionKeys, SessionPrincipal, SessionSigningKeys, SessionTokenKind,
};

use super::GrantIssuer;

const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef";
const REPLAYED: &str = "the authorization code was already used";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn keys() -> SessionSigningKeys {
    SessionSigningKeys::Symmetric(SessionKeys::new(SecretString::from(SIGNING_KEY)).unwrap())
}

struct FixedEntropy;

impl SessionEntropy for FixedEntropy {
    fn jti(&self) -> Result<[u8; 16], Unspecified> {
        Ok([7; 16])
    }
}

struct FlakyEntropy {
    calls: AtomicU32,
}

impl SessionEntropy for FlakyEntropy {
    fn jti(&self) -> Result<[u8; 16], Unspecified> {
        let calls = self.calls.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        if calls == 1 {
            Err(Unspecified)
        } else {
            Ok([7; 16])
        }
    }
}

struct Reload(Option<ReloadUserFailure>);

impl ReloadUser for Reload {
    fn reload_user(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Option<ReloadUserFailure>> + Send {
        ready(self.0)
    }
}

struct Mint {
    failure: Option<ProxyCredentialMintFailure>,
    team_id: Option<String>,
}

impl MintProxyCredential for Mint {
    fn mint(
        &self,
        user_id: &str,
        team_id: Option<&str>,
    ) -> impl Future<Output = Result<MintedProxyCredential, ProxyCredentialMintFailure>> + Send
    {
        let minted = MintedProxyCredential {
            key: SecretString::from("proxy-key-1"),
            expires_in: NonZeroU64::new(600).unwrap(),
            user_id: user_id.to_owned(),
            team_id: self.team_id.clone().or_else(|| team_id.map(str::to_owned)),
        };
        ready(self.failure.map_or_else(|| Ok(minted), Err))
    }
}

struct Exchange {
    answer: Result<SubjectIdentity, SubjectTokenRefusal>,
}

impl Exchange {
    fn identity(user_id: &str, team_id: Option<&str>) -> Self {
        Self {
            answer: Ok(SubjectIdentity {
                user_id: user_id.to_owned(),
                team_id: team_id.map(str::to_owned),
            }),
        }
    }

    fn refusal(description: &str) -> Self {
        Self {
            answer: Err(SubjectTokenRefusal {
                error: RefusalCode::InvalidRequest,
                description: description.to_owned(),
            }),
        }
    }
}

impl ExchangeSubjectToken<()> for Exchange {
    fn exchange(
        &self,
        _subject_token: &str,
        _context: &(),
    ) -> impl Future<Output = Result<SubjectIdentity, SubjectTokenRefusal>> + Send {
        ready(self.answer.clone())
    }
}

struct Harness {
    keys: SessionSigningKeys,
    entropy: Box<dyn SessionEntropy>,
    guard: SingleUseGuard<NoBackend>,
    reload: Reload,
    mint: Mint,
    exchange: Exchange,
    resource: Option<&'static str>,
    context: (),
}

impl Harness {
    fn new() -> Self {
        Self {
            keys: keys(),
            entropy: Box::new(FixedEntropy),
            guard: guard(),
            reload: Reload(None),
            mint: Mint {
                failure: None,
                team_id: Some("team-b".to_owned()),
            },
            exchange: Exchange::identity("bob", Some("team-x")),
            resource: None,
            context: (),
        }
    }

    fn issuer(&self) -> GrantIssuer<'_, Reload, Mint, NoBackend, Exchange, ()> {
        GrantIssuer {
            keys: &self.keys,
            resource: self.resource,
            base_url: BASE,
            now: now(),
            entropy: &*self.entropy,
            reload_user: &self.reload,
            mint_proxy_credential: &self.mint,
            exchange_subject_token: &self.exchange,
            exchange_context: &self.context,
            guard: &self.guard,
        }
    }

    async fn issue(&self, principal: SessionPrincipal, key: &str) -> TokenReply {
        self.issuer().issue(principal, key, 120, REPLAYED).await
    }
}

fn alice() -> SessionPrincipal {
    SessionPrincipal {
        user_id: "alice".to_owned(),
        client_id: "client-1".to_owned(),
        resource_server_id: None,
        audience: None,
        team_id: Some("team-a".to_owned()),
    }
}

fn proxy_alice() -> SessionPrincipal {
    SessionPrincipal {
        audience: Some(SessionAudience::ProxyApi),
        ..alice()
    }
}

fn session_pair(reply: TokenReply) -> SessionPairBody {
    match reply {
        TokenReply::Json { status, body, .. } => {
            assert_eq!(status, 200);
            match *body {
                TokenBody::SessionPair(pair) => pair,
                other @ TokenBody::ProxyCredential(_)
                | other @ TokenBody::Revoked(_)
                | other @ TokenBody::Introspection(_) => {
                    panic!("expected a session pair, got {other:?}")
                }
            }
        }
        TokenReply::Error(error) => panic!("expected a session pair, got {}", error.description),
    }
}

fn proxy_body(reply: TokenReply) -> ProxyCredentialBody {
    match reply {
        TokenReply::Json { status, body, .. } => {
            assert_eq!(status, 200);
            match *body {
                TokenBody::ProxyCredential(credential) => credential,
                other @ TokenBody::SessionPair(_)
                | other @ TokenBody::Revoked(_)
                | other @ TokenBody::Introspection(_) => {
                    panic!("expected a proxy credential, got {other:?}")
                }
            }
        }
        TokenReply::Error(error) => {
            panic!("expected a proxy credential, got {}", error.description)
        }
    }
}

fn error(reply: TokenReply) -> OauthError {
    match reply {
        TokenReply::Error(error) => error,
        TokenReply::Json { status, .. } => panic!("expected an error, got status {status}"),
    }
}

#[tokio::test]
async fn a_session_grant_issues_a_pair_with_the_hour_ttl() {
    let harness = Harness::new();
    let pair = session_pair(harness.issue(alice(), "code:1").await);
    assert_eq!(pair.token_type, "Bearer");
    assert_eq!(pair.expires_in, SESSION_TTL_SECONDS);
    let access = open_session_token(
        &pair.access_token,
        SessionTokenKind::Session,
        &harness.keys,
        now(),
    )
    .unwrap();
    assert_eq!(access.principal, alice());
    assert_eq!(access.exp.saturating_sub(access.iat), SESSION_TTL_SECONDS);
    let refresh = open_session_token(
        &pair.refresh_token,
        SessionTokenKind::Refresh,
        &harness.keys,
        now(),
    )
    .unwrap();
    assert_eq!(refresh.principal, alice());
    assert_eq!(
        refresh.exp.saturating_sub(refresh.iat),
        SESSION_REFRESH_TTL_SECONDS
    );
}

#[rstest]
#[case(ReloadUserFailure::Unavailable, 503)]
#[case(ReloadUserFailure::Faulted, 503)]
#[case(ReloadUserFailure::Unresolvable, 500)]
#[case(ReloadUserFailure::NoActiveKey, 400)]
#[tokio::test]
async fn a_reload_failure_is_refused_before_the_claim(
    #[case] failure: ReloadUserFailure,
    #[case] status: u16,
) {
    let mut harness = Harness::new();
    harness.reload = Reload(Some(failure));
    assert_eq!(error(harness.issue(alice(), "code:1").await).status, status);
    harness.reload = Reload(None);
    session_pair(harness.issue(alice(), "code:1").await);
}

#[tokio::test]
async fn a_replayed_code_is_refused() {
    let harness = Harness::new();
    session_pair(harness.issue(alice(), "code:1").await);
    let refusal = error(harness.issue(alice(), "code:1").await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidGrant);
    assert_eq!(refusal.description, REPLAYED);
}

#[tokio::test]
async fn a_claim_outage_is_a_503() {
    let mut harness = Harness::new();
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    let refusal = error(harness.issue(alice(), "code:1").await);
    assert_eq!(refusal.status, 503);
    assert_eq!(refusal.error, OauthErrorCode::TemporarilyUnavailable);
}

// the session path claims the code before minting, so a mint failure leaves the code burnt
#[tokio::test]
async fn a_mint_failure_keeps_the_claim() {
    let mut harness = Harness::new();
    harness.entropy = Box::new(FlakyEntropy {
        calls: AtomicU32::new(0),
    });
    let failure = error(harness.issue(alice(), "code:1").await);
    assert_eq!(failure.status, 500);
    assert_eq!(failure.description, "failed to mint the session credential");
    assert_eq!(
        error(harness.issue(alice(), "code:1").await).description,
        REPLAYED
    );
}

// the proxy path mints first, so a refused mint leaves the claim open for a retry
#[tokio::test]
async fn a_mint_failure_before_the_claim_does_not_burn_it() {
    let mut harness = Harness::new();
    harness.mint.failure = Some(ProxyCredentialMintFailure::TeamRequired);
    let refusal = error(harness.issue(proxy_alice(), "code:1").await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidGrant);
    harness.mint.failure = None;
    proxy_body(harness.issue(proxy_alice(), "code:1").await);
}

#[tokio::test]
async fn a_proxy_grant_binds_the_minted_team_into_the_refresh_token() {
    let harness = Harness::new();
    let body = proxy_body(harness.issue(proxy_alice(), "code:1").await);
    assert_eq!(body.access_token, "proxy-key-1");
    assert_eq!(body.token_type, "Bearer");
    assert_eq!(body.expires_in, 600);
    assert_eq!(body.user_id, "alice");
    assert_eq!(body.team_id.as_deref(), Some("team-b"));
    assert_eq!(body.issued_token_type, None);
    let refresh = open_session_token(
        &body.refresh_token,
        SessionTokenKind::Refresh,
        &harness.keys,
        now(),
    )
    .unwrap();
    assert_eq!(refresh.principal.team_id.as_deref(), Some("team-b"));
    assert_eq!(refresh.principal.audience, Some(SessionAudience::ProxyApi));
    assert_eq!(refresh.principal.client_id, "client-1");
}

#[tokio::test]
async fn a_proxy_grant_without_a_team_keeps_a_null_team() {
    let mut harness = Harness::new();
    harness.mint.team_id = None;
    let mut principal = proxy_alice();
    principal.team_id = None;
    let body = proxy_body(harness.issue(principal, "code:1").await);
    assert_eq!(body.team_id, None);
    let refresh = open_session_token(
        &body.refresh_token,
        SessionTokenKind::Refresh,
        &harness.keys,
        now(),
    )
    .unwrap();
    assert_eq!(refresh.principal.team_id, None);
}

#[tokio::test]
async fn a_proxy_grant_does_not_reload_the_user() {
    let mut harness = Harness::new();
    harness.reload = Reload(Some(ReloadUserFailure::Unavailable));
    proxy_body(harness.issue(proxy_alice(), "code:1").await);
}

#[tokio::test]
async fn a_proxy_grant_accepts_the_proxy_api_resource() {
    let mut harness = Harness::new();
    harness.resource = Some(BASE);
    proxy_body(harness.issue(proxy_alice(), "code:1").await);
}

#[tokio::test]
async fn a_proxy_grant_refuses_another_resource() {
    let mut harness = Harness::new();
    harness.resource = Some("https://proxy.test:8443/other/mcp");
    let refusal = error(harness.issue(proxy_alice(), "code:1").await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidTarget);
    assert_eq!(
        refusal.description,
        "resource does not match the proxy API this grant was issued for"
    );
}

#[tokio::test]
async fn an_exchange_issues_a_proxy_credential_naming_the_type() {
    let mut harness = Harness::new();
    harness.mint.team_id = None;
    let body = proxy_body(harness.issuer().exchange("idp-token", "client-1").await);
    assert_eq!(body.access_token, "proxy-key-1");
    assert_eq!(body.user_id, "bob");
    assert_eq!(body.team_id.as_deref(), Some("team-x"));
    assert_eq!(body.issued_token_type, Some(ACCESS_TOKEN_TOKEN_TYPE));
    let refresh = open_session_token(
        &body.refresh_token,
        SessionTokenKind::Refresh,
        &harness.keys,
        now(),
    )
    .unwrap();
    assert_eq!(refresh.principal.user_id, "bob");
    assert_eq!(refresh.principal.team_id.as_deref(), Some("team-x"));
    assert_eq!(refresh.principal.audience, Some(SessionAudience::ProxyApi));
}

#[tokio::test]
async fn an_exchange_refusal_keeps_the_idp_text() {
    let mut harness = Harness::new();
    harness.exchange = Exchange::refusal("the IdP rejected the assertion");
    let refusal = error(harness.issuer().exchange("idp-token", "client-1").await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(refusal.description, "the IdP rejected the assertion");
}

#[tokio::test]
async fn an_exchange_never_takes_a_claim() {
    let mut harness = Harness::new();
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    proxy_body(harness.issuer().exchange("idp-token", "client-1").await);
}

#[tokio::test]
async fn an_exchange_refuses_another_resource_before_the_idp() {
    let mut harness = Harness::new();
    harness.resource = Some("https://proxy.test:8443/other/mcp");
    let refusal = error(harness.issuer().exchange("idp-token", "client-1").await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidTarget);
}
