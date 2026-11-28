#![cfg(test)]

use std::future::{Future, ready};
use std::num::NonZeroU64;
use std::sync::Mutex;

use aws_lc_rs::error::Unspecified;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::stored_cipher::DbCipher;
use chrono::{DateTime, TimeZone as _, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::errors::SessionKeyError;
use crate::gateway_authorize::tests::support::{
    BASE, CALLBACK, M2M_SERVER, NoBackend, OAUTH_SERVER, SALT, cipher, client_id as sealed_client,
    guard, registry,
};
use crate::gateway_dcr::contracts::{
    ACCESS_TOKEN_TOKEN_TYPE, GATEWAY_AUTH_CODE_PREFIX, SUBJECT_TOKEN_TYPES,
    TOKEN_EXCHANGE_GRANT_TYPE,
};
use crate::gateway_dcr::models::GatewayAuthCode;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::ports::{
    MintProxyCredential, MintedProxyCredential, ProxyCredentialMintFailure, ReloadUser,
    ReloadUserFailure,
};
use crate::gateway_dcr::sealed::seal;
use crate::gateway_dcr::single_use::{ClaimAuthority, SingleUseGuard};
use crate::gateway_dcr::subject_token::{
    ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal,
};
use crate::gateway_token::support::reply::{
    ProxyCredentialBody, SessionPairBody, TokenBody, TokenReply,
};
use crate::gateway_token::support::{SessionEntropy, SigningSetup, signing_refusal};
use crate::registry::servers::ServerRegistry;
use crate::session_credentials::SessionSigningConfigError;
use crate::session_tokens::{SESSION_TTL_SECONDS, mint_session_token, open_session_token};
use crate::types::{
    SessionAudience, SessionKeys, SessionPrincipal, SessionSigningKeys, SessionTokenKind,
};

use super::{TokenForm, TokenPorts, aggregate_token};

const NOW: i64 = 1_700_000_000;
const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef";
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
const CHALLENGE: &str = "RqIZl4LIgn8KxW9QO-nTnv7pf0CnNrksx9fF-CXP2FE";
const WRONG_VERIFIER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
const REQUIRED: &str = "code, redirect_uri, and code_verifier are required";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(NOW, 0).unwrap()
}

fn challenge_of(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

struct CountingEntropy {
    calls: Mutex<u8>,
}

impl SessionEntropy for CountingEntropy {
    fn jti(&self) -> Result<[u8; 16], Unspecified> {
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        Ok([*calls; 16])
    }
}

struct FlakyEntropy {
    calls: Mutex<u8>,
}

impl SessionEntropy for FlakyEntropy {
    fn jti(&self) -> Result<[u8; 16], Unspecified> {
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        if *calls == 1 {
            Err(Unspecified)
        } else {
            Ok([*calls; 16])
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
        let answer = match self.failure {
            Some(failure) => Err(failure),
            None => Ok(MintedProxyCredential {
                key: SecretString::from("proxy-key-1"),
                expires_in: NonZeroU64::new(600).unwrap(),
                user_id: user_id.to_owned(),
                team_id: self.team_id.clone().or_else(|| team_id.map(str::to_owned)),
            }),
        };
        ready(answer)
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

    fn refusal(error: RefusalCode, description: &str) -> Self {
        Self {
            answer: Err(SubjectTokenRefusal {
                error,
                description: description.to_owned(),
            }),
        }
    }

    fn unconfigured() -> Self {
        Self::refusal(
            RefusalCode::UnsupportedGrantType,
            "this gateway is not configured to exchange IdP tokens",
        )
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
    setup: SigningSetup,
    cipher: DbCipher,
    registry: ServerRegistry,
    entropy: Box<dyn SessionEntropy>,
    guard: SingleUseGuard<NoBackend>,
    reload: Reload,
    mint: Mint,
    exchange: Exchange,
    context: (),
}

impl Harness {
    fn new() -> Self {
        Self {
            setup: SigningSetup::Ready(SessionSigningKeys::Symmetric(
                SessionKeys::new(SecretString::from(SIGNING_KEY)).unwrap(),
            )),
            cipher: cipher(SALT),
            registry: registry(&[OAUTH_SERVER, M2M_SERVER]),
            entropy: Box::new(CountingEntropy {
                calls: Mutex::new(0),
            }),
            guard: guard(),
            reload: Reload(None),
            mint: Mint {
                failure: None,
                team_id: None,
            },
            exchange: Exchange::identity("bob", Some("team-x")),
            context: (),
        }
    }

    fn keys(&self) -> &SessionSigningKeys {
        signing_refusal(&self.setup).unwrap()
    }

    fn sealed_code(&self, code: &GatewayAuthCode) -> String {
        seal(GATEWAY_AUTH_CODE_PREFIX, code, &self.cipher).unwrap()
    }

    fn refresh(&self, principal: &SessionPrincipal) -> String {
        mint_session_token(
            principal,
            SessionTokenKind::Refresh,
            self.keys(),
            now(),
            || Ok([9; 16]),
        )
        .unwrap()
        .token
        .expose_secret()
        .to_owned()
    }

    fn access(&self, principal: &SessionPrincipal) -> String {
        mint_session_token(
            principal,
            SessionTokenKind::Session,
            self.keys(),
            now(),
            || Ok([8; 16]),
        )
        .unwrap()
        .token
        .expose_secret()
        .to_owned()
    }

    async fn token(&self, form: &TokenForm<'_>) -> TokenReply {
        let ports = TokenPorts {
            registry: &self.registry,
            cipher: &self.cipher,
            base_url: BASE,
            entropy: &*self.entropy,
            reload_user: &self.reload,
            mint_proxy_credential: &self.mint,
            guard: &self.guard,
            exchange_subject_token: &self.exchange,
            exchange_context: &self.context,
        };
        aggregate_token(&self.setup, form, &ports, now()).await
    }
}

fn form<'form>(grant_type: &'form str) -> TokenForm<'form> {
    TokenForm {
        grant_type: Some(grant_type),
        code: None,
        redirect_uri: None,
        client_id: None,
        code_verifier: None,
        refresh_token: None,
        resource: None,
        subject_token: None,
        subject_token_type: None,
        requested_token_type: None,
    }
}

fn auth_code(challenge: &str, jti: &str) -> GatewayAuthCode {
    GatewayAuthCode {
        user_id: "alice".to_owned(),
        client_id: "client-1".to_owned(),
        redirect_uri: CALLBACK.to_owned(),
        code_challenge: challenge.to_owned(),
        jti: jti.to_owned(),
        iat: NOW,
        exp: NOW + 60,
        resource_server_id: None,
        audience: None,
        team_id: Some("team-a".to_owned()),
    }
}

fn code_form<'form>(code: &'form str) -> TokenForm<'form> {
    let mut request = form("authorization_code");
    request.code = Some(code);
    request.redirect_uri = Some(CALLBACK);
    request.client_id = Some("client-1");
    request.code_verifier = Some(VERIFIER);
    request
}

fn refresh_form<'form>(token: &'form str) -> TokenForm<'form> {
    let mut request = form("refresh_token");
    request.refresh_token = Some(token);
    request.client_id = Some("client-1");
    request
}

fn exchange_form<'form>(client: &'form str) -> TokenForm<'form> {
    let mut request = form(TOKEN_EXCHANGE_GRANT_TYPE);
    request.client_id = Some(client);
    request.subject_token = Some("idp-token");
    request.subject_token_type = Some(SUBJECT_TOKEN_TYPES[0]);
    request
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

fn session_pair(reply: TokenReply) -> SessionPairBody {
    match reply {
        TokenReply::Json { status, body, .. } => {
            assert_eq!(status, 200);
            match *body {
                TokenBody::SessionPair(pair) => pair,
                other => panic!("expected a session pair, got {other:?}"),
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
                TokenBody::ProxyCredential(body) => body,
                other => panic!("expected a proxy credential, got {other:?}"),
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
async fn an_unknown_grant_type_is_refused() {
    let harness = Harness::new();
    let refusal = error(harness.token(&form("password")).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::UnsupportedGrantType);
    let description = format!(
        "grant_type must be authorization_code, refresh_token, or {TOKEN_EXCHANGE_GRANT_TYPE}"
    );
    assert_eq!(refusal.description, description);
    let mut absent = form("authorization_code");
    absent.grant_type = None;
    assert_eq!(error(harness.token(&absent).await).description, description);
}

#[tokio::test]
async fn a_signing_defect_is_refused_before_the_grant() {
    let mut harness = Harness::new();
    harness.setup = SigningSetup::NoMasterKey;
    let refusal = error(harness.token(&form("password")).await);
    assert_eq!(refusal.status, 500);
    assert_eq!(
        refusal.description,
        "the gateway has no master key configured"
    );
    harness.setup =
        SigningSetup::Invalid(SessionSigningConfigError::Keys(SessionKeyError::EmptyKid));
    assert_eq!(
        error(harness.token(&form("password")).await).description,
        "the gateway session signing configuration is invalid"
    );
}

#[tokio::test]
async fn a_code_grant_needs_all_three_fields() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let mut missing_code = code_form(&code);
    missing_code.code = None;
    assert_eq!(
        error(harness.token(&missing_code).await).description,
        REQUIRED
    );
    let mut missing_redirect = code_form(&code);
    missing_redirect.redirect_uri = None;
    assert_eq!(
        error(harness.token(&missing_redirect).await).description,
        REQUIRED
    );
    let mut missing_verifier = code_form(&code);
    missing_verifier.code_verifier = None;
    assert_eq!(
        error(harness.token(&missing_verifier).await).description,
        REQUIRED
    );
}

#[tokio::test]
async fn a_verifier_length_is_checked() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let mut short = code_form(&code);
    let short_verifier = "a".repeat(42);
    short.code_verifier = Some(&short_verifier);
    let refusal = error(harness.token(&short).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(
        refusal.description,
        "code_verifier must be 43 to 128 characters (RFC 7636)"
    );
    let mut long = code_form(&code);
    let long_verifier = "a".repeat(129);
    long.code_verifier = Some(&long_verifier);
    assert_eq!(
        error(harness.token(&long).await).description,
        refusal.description
    );
}

#[tokio::test]
async fn a_verifier_at_each_bound_is_accepted() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    session_pair(harness.token(&code_form(&code)).await);
    let long_verifier = "a".repeat(128);
    let challenge = challenge_of(&long_verifier);
    let other = harness.sealed_code(&auth_code(&challenge, "jti-2"));
    let mut long = code_form(&other);
    long.code_verifier = Some(&long_verifier);
    session_pair(harness.token(&long).await);
}

#[tokio::test]
async fn an_invalid_code_is_refused() {
    let harness = Harness::new();
    let refusal = error(harness.token(&code_form("llm_gcode_bogus")).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidGrant);
    assert_eq!(refusal.description, "the authorization code is invalid");
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let harness = Harness::new();
    let mut expired = auth_code(CHALLENGE, "jti-1");
    expired.exp = NOW;
    let code = harness.sealed_code(&expired);
    let refusal = error(harness.token(&code_form(&code)).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidGrant);
    assert_eq!(refusal.description, "the authorization code has expired");
}

#[tokio::test]
async fn a_code_for_another_client_is_refused() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let mut other_client = code_form(&code);
    other_client.client_id = Some("client-2");
    let refusal = error(harness.token(&other_client).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidGrant);
    assert_eq!(
        refusal.description,
        "the authorization code was issued to a different client"
    );
    let mut other_redirect = code_form(&code);
    other_redirect.redirect_uri = Some("http://127.0.0.1:9999/callback");
    assert_eq!(
        error(harness.token(&other_redirect).await).description,
        refusal.description
    );
}

#[tokio::test]
async fn a_wrong_pkce_verifier_is_refused() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let mut wrong = code_form(&code);
    wrong.code_verifier = Some(WRONG_VERIFIER);
    let refusal = error(harness.token(&wrong).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidGrant);
    assert_eq!(refusal.description, "PKCE verification failed");
}

#[tokio::test]
async fn a_code_grant_issues_a_pair_and_burns_the_code() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let pair = session_pair(harness.token(&code_form(&code)).await);
    assert_eq!(pair.expires_in, SESSION_TTL_SECONDS);
    let opened = open_session_token(
        &pair.access_token,
        SessionTokenKind::Session,
        harness.keys(),
        now(),
    )
    .unwrap();
    assert_eq!(opened.principal, alice());
    let refusal = error(harness.token(&code_form(&code)).await);
    assert_eq!(
        refusal.description,
        "the authorization code was already used"
    );
}

#[tokio::test]
async fn a_code_grant_claims_before_minting() {
    let mut harness = Harness::new();
    harness.entropy = Box::new(FlakyEntropy {
        calls: Mutex::new(0),
    });
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    assert_eq!(error(harness.token(&code_form(&code)).await).status, 500);
    assert_eq!(
        error(harness.token(&code_form(&code)).await).description,
        "the authorization code was already used"
    );
}

#[tokio::test]
async fn a_code_claim_outage_is_a_503() {
    let mut harness = Harness::new();
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let refusal = error(harness.token(&code_form(&code)).await);
    assert_eq!(refusal.status, 503);
    assert_eq!(refusal.error, OauthErrorCode::TemporarilyUnavailable);
}

#[tokio::test]
async fn a_scoped_code_keeps_its_server() {
    let harness = Harness::new();
    let mut scoped = auth_code(CHALLENGE, "jti-1");
    scoped.resource_server_id = Some("srv-a".to_owned());
    let code = harness.sealed_code(&scoped);
    let mut matching = code_form(&code);
    matching.resource = Some("https://proxy.test:8443/alpha/mcp");
    session_pair(harness.token(&matching).await);
    let mut other = auth_code(CHALLENGE, "jti-2");
    other.resource_server_id = Some("srv-a".to_owned());
    let other_code = harness.sealed_code(&other);
    let mut conflicting = code_form(&other_code);
    conflicting.resource = Some("https://proxy.test:8443/beta/mcp");
    let refusal = error(harness.token(&conflicting).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidTarget);
    assert_eq!(
        refusal.description,
        "resource does not match the scope this code was issued for"
    );
}

#[tokio::test]
async fn a_code_grant_checks_in_litellm_order() {
    let harness = Harness::new();
    let mut expired = auth_code(CHALLENGE, "jti-1");
    expired.exp = NOW;
    let code = harness.sealed_code(&expired);
    let mut sent = code_form(&code);
    sent.client_id = Some("client-2");
    sent.code_verifier = Some(WRONG_VERIFIER);
    assert_eq!(
        error(harness.token(&sent).await).description,
        "the authorization code has expired"
    );
    let live = harness.sealed_code(&auth_code(CHALLENGE, "jti-2"));
    let mut wrong_client = code_form(&live);
    wrong_client.client_id = Some("client-2");
    wrong_client.code_verifier = Some(WRONG_VERIFIER);
    assert_eq!(
        error(harness.token(&wrong_client).await).description,
        "the authorization code was issued to a different client"
    );
    let mut scoped = auth_code(CHALLENGE, "jti-3");
    scoped.resource_server_id = Some("srv-a".to_owned());
    let scoped_code = harness.sealed_code(&scoped);
    let mut conflicting = code_form(&scoped_code);
    conflicting.resource = Some("https://proxy.test:8443/beta/mcp");
    conflicting.code_verifier = Some(WRONG_VERIFIER);
    assert_eq!(
        error(harness.token(&conflicting).await).description,
        "resource does not match the scope this code was issued for"
    );
}

#[tokio::test]
async fn an_unscoped_code_ignores_the_resource() {
    let harness = Harness::new();
    let code = harness.sealed_code(&auth_code(CHALLENGE, "jti-1"));
    let mut request = code_form(&code);
    request.resource = Some("https://proxy.test:8443/beta/mcp");
    session_pair(harness.token(&request).await);
}

#[tokio::test]
async fn a_proxy_api_code_mints_a_credential() {
    let harness = Harness::new();
    let mut code = auth_code(CHALLENGE, "jti-1");
    code.audience = Some(SessionAudience::ProxyApi);
    let sealed = harness.sealed_code(&code);
    let body = proxy_body(harness.token(&code_form(&sealed)).await);
    assert_eq!(body.access_token, "proxy-key-1");
    assert_eq!(body.user_id, "alice");
    assert_eq!(body.team_id.as_deref(), Some("team-a"));
    let mut other = auth_code(CHALLENGE, "jti-2");
    other.audience = Some(SessionAudience::ProxyApi);
    let other_code = harness.sealed_code(&other);
    let mut conflicting = code_form(&other_code);
    conflicting.resource = Some("https://proxy.test:8443/beta/mcp");
    let refusal = error(harness.token(&conflicting).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidTarget);
    assert_eq!(
        refusal.description,
        "resource does not match the proxy API this grant was issued for"
    );
}

#[tokio::test]
async fn a_refresh_grant_rotates_the_pair() {
    let harness = Harness::new();
    let refresh = harness.refresh(&alice());
    let pair = session_pair(harness.token(&refresh_form(&refresh)).await);
    assert_eq!(pair.expires_in, SESSION_TTL_SECONDS);
    assert_ne!(pair.refresh_token, refresh);
    let opened = open_session_token(
        &pair.refresh_token,
        SessionTokenKind::Refresh,
        harness.keys(),
        now(),
    )
    .unwrap();
    assert_eq!(opened.principal, alice());
    let refusal = error(harness.token(&refresh_form(&refresh)).await);
    assert_eq!(refusal.description, "the refresh token was already used");
}

#[tokio::test]
async fn a_refresh_grant_needs_its_fields() {
    let harness = Harness::new();
    let mut absent = form("refresh_token");
    absent.client_id = Some("client-1");
    let refusal = error(harness.token(&absent).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(refusal.description, "refresh_token is required");
    let refresh = harness.refresh(&alice());
    let mut other_client = refresh_form(&refresh);
    other_client.client_id = Some("client-2");
    assert_eq!(
        error(harness.token(&other_client).await).description,
        "the refresh token is invalid for this client"
    );
    let access = harness.access(&alice());
    assert_eq!(
        error(harness.token(&refresh_form(&access)).await).description,
        "the refresh token is invalid for this client"
    );
}

#[tokio::test]
async fn a_scoped_refresh_token_keeps_its_server() {
    let harness = Harness::new();
    let mut scoped = alice();
    scoped.resource_server_id = Some("srv-a".to_owned());
    let refresh = harness.refresh(&scoped);
    let mut conflicting = refresh_form(&refresh);
    conflicting.resource = Some("https://proxy.test:8443/beta/mcp");
    let refusal = error(harness.token(&conflicting).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidTarget);
    assert_eq!(
        refusal.description,
        "resource does not match the scope this token was issued for"
    );
    // the resource conflict is refused before the claim, so the same token still works
    let mut matching = refresh_form(&refresh);
    matching.resource = Some("https://proxy.test:8443/alpha/mcp");
    session_pair(harness.token(&matching).await);
}

#[tokio::test]
async fn a_refresh_claim_outage_is_a_503() {
    let mut harness = Harness::new();
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    let refresh = harness.refresh(&alice());
    assert_eq!(
        error(harness.token(&refresh_form(&refresh)).await).status,
        503
    );
}

#[tokio::test]
async fn an_exchange_needs_a_registered_client() {
    let harness = Harness::new();
    let refusal = error(harness.token(&exchange_form("llm_dcrc_bogus")).await);
    assert_eq!(refusal.status, 401);
    assert_eq!(refusal.error, OauthErrorCode::InvalidClient);
    let mut absent = exchange_form("llm_dcrc_bogus");
    absent.client_id = None;
    assert_eq!(error(harness.token(&absent).await).status, 401);
}

#[tokio::test]
async fn an_exchange_needs_both_subject_fields() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let mut missing_token = exchange_form(&client);
    missing_token.subject_token = None;
    let refusal = error(harness.token(&missing_token).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(
        refusal.description,
        "subject_token and subject_token_type are required"
    );
    let mut missing_type = exchange_form(&client);
    missing_type.subject_token_type = None;
    assert_eq!(
        error(harness.token(&missing_type).await).description,
        refusal.description
    );
}

#[tokio::test]
async fn an_exchange_sorts_the_accepted_types() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let mut wrong = exchange_form(&client);
    wrong.subject_token_type = Some("urn:example:other");
    let refusal = error(harness.token(&wrong).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(
        refusal.description,
        "subject_token_type must be one of urn:ietf:params:oauth:token-type:access_token, urn:ietf:params:oauth:token-type:id_token, urn:ietf:params:oauth:token-type:jwt"
    );
}

#[tokio::test]
async fn an_exchange_refuses_another_requested_type() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let mut wrong = exchange_form(&client);
    wrong.requested_token_type = Some("urn:ietf:params:oauth:token-type:jwt");
    let refusal = error(harness.token(&wrong).await);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(
        refusal.description,
        "requested_token_type must be urn:ietf:params:oauth:token-type:access_token"
    );
    let mut accepted = exchange_form(&client);
    accepted.requested_token_type = Some(ACCESS_TOKEN_TOKEN_TYPE);
    proxy_body(harness.token(&accepted).await);
}

#[tokio::test]
async fn an_exchange_issues_a_proxy_credential() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let body = proxy_body(harness.token(&exchange_form(&client)).await);
    assert_eq!(body.access_token, "proxy-key-1");
    assert_eq!(body.user_id, "bob");
    assert_eq!(body.team_id.as_deref(), Some("team-x"));
    assert_eq!(body.issued_token_type, Some(ACCESS_TOKEN_TOKEN_TYPE));
    // an exchange takes no claim, so the same subject token works twice
    proxy_body(harness.token(&exchange_form(&client)).await);
}

#[tokio::test]
async fn an_exchange_refuses_another_resource() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let mut request = exchange_form(&client);
    request.resource = Some("https://proxy.test:8443/other/mcp");
    let refusal = error(harness.token(&request).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidTarget);
}

#[tokio::test]
async fn an_unconfigured_exchange_is_unsupported() {
    let mut harness = Harness::new();
    harness.exchange = Exchange::unconfigured();
    let client = sealed_client(&[CALLBACK]);
    let refusal = error(harness.token(&exchange_form(&client)).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::UnsupportedGrantType);
    assert_eq!(
        refusal.description,
        "this gateway is not configured to exchange IdP tokens"
    );
}

#[tokio::test]
async fn an_exchange_refusal_keeps_the_idp_text() {
    let mut harness = Harness::new();
    harness.exchange = Exchange::refusal(RefusalCode::InvalidRequest, "the IdP rejected it");
    let client = sealed_client(&[CALLBACK]);
    let refusal = error(harness.token(&exchange_form(&client)).await);
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.error, OauthErrorCode::InvalidRequest);
    assert_eq!(refusal.description, "the IdP rejected it");
}
