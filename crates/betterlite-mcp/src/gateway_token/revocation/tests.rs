#![cfg(test)]

use betterlite_db::stored_cipher::DbCipher;
use chrono::{DateTime, TimeZone as _, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use crate::errors::SessionKeyError;
use crate::gateway_authorize::tests::support::{
    CALLBACK, NoBackend, SALT, cipher, client_id as sealed_client, guard,
};
use crate::gateway_dcr::contracts::REFRESH_CLAIM_KEY_PREFIX;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::single_use::{
    CLAIM_UNAVAILABLE_DESCRIPTION, ClaimAuthority, PeekOutcome, SingleUseGuard,
};
use crate::gateway_token::support::reply::{TokenBody, TokenReply};
use crate::gateway_token::support::{SigningSetup, signing_refusal};
use crate::session_credentials::SessionSigningConfigError;
use crate::session_tokens::mint_session_token;
use crate::types::{SessionKeys, SessionPrincipal, SessionSigningKeys, SessionTokenKind};

use super::{RevokeRequest, revoke_refresh_token};

const NOW: i64 = 1_700_000_000;
const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef";
const JTI: &str = "BwcHBwcHBwcHBwcHBwcHBw";
const ACCESS_JTI: &str = "CAgICAgICAgICAgICAgICA";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(NOW, 0).unwrap()
}

fn alice(client: &str) -> SessionPrincipal {
    SessionPrincipal {
        user_id: "alice".to_owned(),
        client_id: client.to_owned(),
        resource_server_id: None,
        audience: None,
        team_id: Some("team-a".to_owned()),
    }
}

struct Harness {
    setup: SigningSetup,
    cipher: DbCipher,
    guard: SingleUseGuard<NoBackend>,
}

impl Harness {
    fn new() -> Self {
        Self {
            setup: SigningSetup::Ready(SessionSigningKeys::Symmetric(
                SessionKeys::new(SecretString::from(SIGNING_KEY)).unwrap(),
            )),
            cipher: cipher(SALT),
            guard: guard(),
        }
    }

    fn keys(&self) -> &SessionSigningKeys {
        signing_refusal(&self.setup).unwrap()
    }

    fn minted(
        &self,
        principal: &SessionPrincipal,
        kind: SessionTokenKind,
        at: DateTime<Utc>,
        byte: u8,
    ) -> String {
        mint_session_token(principal, kind, self.keys(), at, || Ok([byte; 16]))
            .unwrap()
            .token
            .expose_secret()
            .to_owned()
    }

    async fn revoke(&self, token: &str, client: Option<&str>, at: DateTime<Utc>) -> TokenReply {
        let request = RevokeRequest {
            token,
            client_id: client,
        };
        revoke_refresh_token(&request, &self.setup, &self.guard, &self.cipher, at).await
    }

    async fn claim_status(&self, jti: &str) -> PeekOutcome {
        let key = format!("{REFRESH_CLAIM_KEY_PREFIX}{jti}");
        self.guard.peek(&key).await
    }
}

fn revoked_json(reply: TokenReply) -> String {
    match reply {
        TokenReply::Json { status, body, .. } => {
            assert_eq!(status, 200);
            match *body {
                TokenBody::Revoked(revoked) => serde_json::to_string(&revoked).unwrap(),
                other @ (TokenBody::SessionPair(_)
                | TokenBody::ProxyCredential(_)
                | TokenBody::Introspection(_)) => {
                    panic!("expected a revoked body, got {other:?}")
                }
            }
        }
        TokenReply::Error(error) => panic!("expected a revoked reply, got {}", error.description),
    }
}

fn error(reply: TokenReply) -> OauthError {
    match reply {
        TokenReply::Error(error) => error,
        TokenReply::Json { status, .. } => panic!("expected an error, got status {status}"),
    }
}

#[tokio::test]
async fn a_live_refresh_token_is_burned() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let token = harness.minted(&alice(&client), SessionTokenKind::Refresh, now(), 7);
    let reply = harness.revoke(&token, Some(&client), now()).await;
    assert_eq!(revoked_json(reply), "{}");
    assert_eq!(harness.claim_status(JTI).await, PeekOutcome::Claimed);
}

#[tokio::test]
async fn a_bearer_prefixed_refresh_is_burned() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let token = format!(
        "Bearer {}",
        harness.minted(&alice(&client), SessionTokenKind::Refresh, now(), 7)
    );
    revoked_json(harness.revoke(&token, Some(&client), now()).await);
    assert_eq!(harness.claim_status(JTI).await, PeekOutcome::Claimed);
}

#[tokio::test]
async fn a_garbage_token_is_still_200() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let reply = harness.revoke("nonsense", Some(&client), now()).await;
    assert_eq!(revoked_json(reply), "{}");
}

#[tokio::test]
async fn an_access_token_is_left_alone() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let token = harness.minted(&alice(&client), SessionTokenKind::Session, now(), 8);
    revoked_json(harness.revoke(&token, Some(&client), now()).await);
    assert_eq!(
        harness.claim_status(ACCESS_JTI).await,
        PeekOutcome::Unclaimed
    );
}

#[tokio::test]
async fn an_expired_refresh_token_is_not_burned() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let token = harness.minted(
        &alice(&client),
        SessionTokenKind::Refresh,
        Utc.timestamp_opt(0, 0).unwrap(),
        7,
    );
    revoked_json(harness.revoke(&token, Some(&client), now()).await);
    assert_eq!(harness.claim_status(JTI).await, PeekOutcome::Unclaimed);
}

#[tokio::test]
async fn another_clients_refresh_token_is_kept() {
    let harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let other = sealed_client(&[CALLBACK]);
    let token = harness.minted(&alice(&other), SessionTokenKind::Refresh, now(), 7);
    revoked_json(harness.revoke(&token, Some(&client), now()).await);
    assert_eq!(harness.claim_status(JTI).await, PeekOutcome::Unclaimed);
}

#[tokio::test]
async fn an_unknown_client_is_refused() {
    let harness = Harness::new();
    let refusal = error(
        harness
            .revoke("nonsense", Some("llm_dcrc_bogus"), now())
            .await,
    );
    assert_eq!(refusal.status, 401);
    assert_eq!(refusal.error, OauthErrorCode::InvalidClient);
    assert_eq!(refusal.description, "unknown or malformed client_id");
    assert_eq!(
        error(harness.revoke("nonsense", None, now()).await).status,
        401
    );
}

#[tokio::test]
async fn a_client_check_precedes_the_signing_check() {
    let mut harness = Harness::new();
    harness.setup = SigningSetup::NoMasterKey;
    let refusal = error(
        harness
            .revoke("nonsense", Some("llm_dcrc_bogus"), now())
            .await,
    );
    assert_eq!(refusal.status, 401);
}

#[tokio::test]
async fn a_missing_master_key_is_a_500() {
    let mut harness = Harness::new();
    harness.setup = SigningSetup::NoMasterKey;
    let client = sealed_client(&[CALLBACK]);
    let refusal = error(harness.revoke("llm_srefresh_x", Some(&client), now()).await);
    assert_eq!(refusal.status, 500);
    assert_eq!(
        refusal.description,
        "the gateway has no master key configured"
    );
}

#[tokio::test]
async fn an_invalid_signing_config_is_a_500() {
    let mut harness = Harness::new();
    harness.setup =
        SigningSetup::Invalid(SessionSigningConfigError::Keys(SessionKeyError::EmptyKid));
    let client = sealed_client(&[CALLBACK]);
    let refusal = error(harness.revoke("llm_srefresh_x", Some(&client), now()).await);
    assert_eq!(
        refusal.description,
        "the gateway session signing configuration is invalid"
    );
}

#[tokio::test]
async fn a_claim_outage_is_a_503() {
    let mut harness = Harness::new();
    let client = sealed_client(&[CALLBACK]);
    let token = harness.minted(&alice(&client), SessionTokenKind::Refresh, now(), 7);
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    let refusal = error(harness.revoke(&token, Some(&client), now()).await);
    assert_eq!(refusal.status, 503);
    assert_eq!(refusal.error, OauthErrorCode::TemporarilyUnavailable);
    assert_eq!(refusal.description, CLAIM_UNAVAILABLE_DESCRIPTION);
    revoked_json(harness.revoke("nonsense", Some(&client), now()).await);
}
