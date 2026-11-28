#![cfg(test)]

use std::future::{Future, ready};

use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::errors::SessionKeyError;
use crate::gateway_authorize::tests::support::{NoBackend, guard};
use crate::gateway_dcr::contracts::{
    DB_FAULTED_DESCRIPTION, DB_UNAVAILABLE_DESCRIPTION, REFRESH_CLAIM_KEY_PREFIX,
};
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::ports::{ReloadUser, ReloadUserFailure};
use crate::gateway_dcr::single_use::{
    CLAIM_UNAVAILABLE_DESCRIPTION, ClaimAuthority, ClaimOutcome, SingleUseGuard,
};
use crate::gateway_token::support::reply::{TokenBody, TokenReply};
use crate::gateway_token::support::{SigningSetup, signing_refusal};
use crate::session_credentials::SessionSigningConfigError;
use crate::session_tokens::mint_session_token;
use crate::types::{
    SessionAudience, SessionKeys, SessionPrincipal, SessionSigningKeys, SessionTokenKind,
};

use super::introspect_gateway_token;

const NOW: i64 = 1_700_000_000;
const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef";
const JTI: &str = "BwcHBwcHBwcHBwcHBwcHBw";
const ACTIVE_SESSION: &str = r#"{"active":true,"iss":"litellm-mcp-gateway","sub":"alice","client_id":"client-1","jti":"BwcHBwcHBwcHBwcHBwcHBw","iat":1700000000,"exp":1700003600,"kind":"session","token_type":"Bearer","team_id":"team-a","resource_server_id":"srv-a","audience":"proxy_api"}"#;
const ACTIVE_REFRESH: &str = r#"{"active":true,"iss":"litellm-mcp-gateway","sub":"alice","client_id":"client-1","jti":"BwcHBwcHBwcHBwcHBwcHBw","iat":1700000000,"exp":1701209600,"kind":"session_refresh","team_id":"team-a"}"#;

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(NOW, 0).unwrap()
}

fn alice(server: Option<&str>, audience: Option<SessionAudience>) -> SessionPrincipal {
    SessionPrincipal {
        user_id: "alice".to_owned(),
        client_id: "client-1".to_owned(),
        resource_server_id: server.map(str::to_owned),
        audience,
        team_id: Some("team-a".to_owned()),
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

struct Harness {
    setup: SigningSetup,
    guard: SingleUseGuard<NoBackend>,
    reload: Reload,
}

impl Harness {
    fn new() -> Self {
        Self {
            setup: SigningSetup::Ready(SessionSigningKeys::Symmetric(
                SessionKeys::new(SecretString::from(SIGNING_KEY)).unwrap(),
            )),
            guard: guard(),
            reload: Reload(None),
        }
    }

    fn keys(&self) -> &SessionSigningKeys {
        signing_refusal(&self.setup).unwrap()
    }

    fn minted(&self, principal: &SessionPrincipal, kind: SessionTokenKind) -> String {
        mint_session_token(principal, kind, self.keys(), now(), || Ok([7; 16]))
            .unwrap()
            .token
            .expose_secret()
            .to_owned()
    }

    async fn introspect(&self, token: &str) -> TokenReply {
        introspect_gateway_token(token, &self.setup, &self.reload, &self.guard, now()).await
    }

    async fn claim_refresh(&self) {
        let key = format!("{REFRESH_CLAIM_KEY_PREFIX}{JTI}");
        assert_eq!(self.guard.claim(&key, 120).await, ClaimOutcome::First);
    }
}

fn introspected_json(reply: TokenReply) -> String {
    match reply {
        TokenReply::Json { status, body, .. } => {
            assert_eq!(status, 200);
            match *body {
                TokenBody::Introspection(introspection) => {
                    serde_json::to_string(&introspection).unwrap()
                }
                other @ TokenBody::SessionPair(_)
                | other @ TokenBody::ProxyCredential(_)
                | other @ TokenBody::Revoked(_) => {
                    panic!("expected an introspection body, got {other:?}")
                }
            }
        }
        TokenReply::Error(error) => panic!("expected an introspection, got {}", error.description),
    }
}

fn error(reply: TokenReply) -> OauthError {
    match reply {
        TokenReply::Error(error) => error,
        TokenReply::Json { status, .. } => panic!("expected an error, got status {status}"),
    }
}

#[tokio::test]
async fn a_garbage_token_is_inactive() {
    let harness = Harness::new();
    assert_eq!(
        introspected_json(harness.introspect("nonsense").await),
        r#"{"active":false}"#
    );
}

#[tokio::test]
async fn a_bearer_prefixed_token_is_inactive() {
    let harness = Harness::new();
    let token = format!(
        "Bearer {}",
        harness.minted(
            &alice(Some("srv-a"), Some(SessionAudience::ProxyApi)),
            SessionTokenKind::Session
        )
    );
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        r#"{"active":false}"#
    );
}

#[tokio::test]
async fn a_tampered_token_is_inactive() {
    let harness = Harness::new();
    let token = format!(
        "{}x",
        harness.minted(
            &alice(Some("srv-a"), Some(SessionAudience::ProxyApi)),
            SessionTokenKind::Session
        )
    );
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        r#"{"active":false}"#
    );
}

#[tokio::test]
async fn an_access_token_reports_its_session_fields() {
    let harness = Harness::new();
    let token = harness.minted(
        &alice(Some("srv-a"), Some(SessionAudience::ProxyApi)),
        SessionTokenKind::Session,
    );
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        ACTIVE_SESSION
    );
}

#[tokio::test]
async fn a_refresh_token_omits_the_token_type() {
    let harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Refresh);
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        ACTIVE_REFRESH
    );
}

#[tokio::test]
async fn an_expired_token_is_inactive() {
    let harness = Harness::new();
    let token = mint_session_token(
        &alice(None, None),
        SessionTokenKind::Session,
        harness.keys(),
        Utc.timestamp_opt(0, 0).unwrap(),
        || Ok([7; 16]),
    )
    .unwrap()
    .token
    .expose_secret()
    .to_owned();
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        r#"{"active":false}"#
    );
}

#[tokio::test]
async fn a_burned_refresh_token_is_inactive() {
    let harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Refresh);
    harness.claim_refresh().await;
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        r#"{"active":false}"#
    );
}

#[tokio::test]
async fn a_refresh_peek_outage_is_a_503() {
    let mut harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Refresh);
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    let refusal = error(harness.introspect(&token).await);
    assert_eq!(refusal.status, 503);
    assert_eq!(refusal.error, OauthErrorCode::TemporarilyUnavailable);
    assert_eq!(refusal.description, CLAIM_UNAVAILABLE_DESCRIPTION);
}

#[tokio::test]
async fn an_access_token_skips_the_claim_lookup() {
    let mut harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Session);
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    introspected_json(harness.introspect(&token).await);
}

#[rstest]
#[case(ReloadUserFailure::Unavailable, DB_UNAVAILABLE_DESCRIPTION)]
#[case(ReloadUserFailure::Faulted, DB_FAULTED_DESCRIPTION)]
#[tokio::test]
async fn a_reload_database_failure_is_a_503(
    #[case] failure: ReloadUserFailure,
    #[case] description: &str,
) {
    let mut harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Session);
    harness.reload = Reload(Some(failure));
    let refusal = error(harness.introspect(&token).await);
    assert_eq!(refusal.status, 503);
    assert_eq!(refusal.description, description);
}

#[rstest]
#[case(ReloadUserFailure::Unresolvable)]
#[case(ReloadUserFailure::NoActiveKey)]
#[tokio::test]
async fn a_vanished_user_is_inactive(#[case] failure: ReloadUserFailure) {
    let mut harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Session);
    harness.reload = Reload(Some(failure));
    assert_eq!(
        introspected_json(harness.introspect(&token).await),
        r#"{"active":false}"#
    );
}

#[tokio::test]
async fn the_claim_check_precedes_the_reload() {
    let mut harness = Harness::new();
    let token = harness.minted(&alice(None, None), SessionTokenKind::Refresh);
    harness.reload = Reload(Some(ReloadUserFailure::NoActiveKey));
    harness.guard = SingleUseGuard::from(ClaimAuthority::Shared(NoBackend));
    assert_eq!(error(harness.introspect(&token).await).status, 503);
}

#[tokio::test]
async fn a_missing_master_key_is_a_500() {
    let mut harness = Harness::new();
    harness.setup = SigningSetup::NoMasterKey;
    let refusal = error(harness.introspect("nonsense").await);
    assert_eq!(refusal.status, 500);
    assert_eq!(refusal.error, OauthErrorCode::ServerError);
    assert_eq!(
        refusal.description,
        "the gateway has no master key configured"
    );
}

#[tokio::test]
async fn an_invalid_signing_config_names_the_defect() {
    let mut harness = Harness::new();
    harness.setup =
        SigningSetup::Invalid(SessionSigningConfigError::Keys(SessionKeyError::EmptyKid));
    let refusal = error(harness.introspect("nonsense").await);
    assert_eq!(refusal.status, 500);
    assert_eq!(refusal.error, OauthErrorCode::ServerError);
    assert_eq!(
        refusal.description,
        "the configured signing keys are invalid"
    );
}
