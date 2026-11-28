#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rstest::rstest;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};

use crate::errors::SessionKeyError;
use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::{
    ACCESS_TOKEN_TOKEN_TYPE, DB_FAULTED_DESCRIPTION, DB_UNAVAILABLE_DESCRIPTION,
};
use crate::gateway_dcr::oauth_error::OauthErrorCode;
use crate::gateway_dcr::ports::{ProxyCredentialMintFailure, ReloadUserFailure};
use crate::gateway_dcr::subject_token::{RefusalCode, SubjectTokenRefusal};
use crate::session_credentials::SessionSigningConfigError;
use crate::types::{SessionAudience, SessionKeys, SessionSigningKeys, SessionTokenKind};

use super::reply::{
    IntrospectionBody, ProxyCredentialBody, RevokedBody, SessionPairBody, TokenBody, TokenReply,
    json_reply,
};
use super::{
    BEARER_TOKEN_TYPE, SigningSetup, failed_to_mint_error, mint_failure_error, no_master_key_error,
    pkce_verifier_matches, reload_failure_error, signing_config_error, signing_refusal,
    subject_refusal_error, unknown_client_error,
};

const SIGNING_KEY: &str = "0123456789abcdef0123456789abcdef";
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
const CHALLENGE: &str = "RqIZl4LIgn8KxW9QO-nTnv7pf0CnNrksx9fF-CXP2FE";

fn keys() -> SessionSigningKeys {
    SessionSigningKeys::Symmetric(SessionKeys::new(SecretString::from(SIGNING_KEY)).unwrap())
}

#[test]
fn pkce_accepts_the_s256_challenge() {
    assert!(pkce_verifier_matches(VERIFIER, CHALLENGE));
}

#[test]
fn pkce_rejects_another_verifier_of_the_same_length() {
    assert!(!pkce_verifier_matches(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq",
        CHALLENGE
    ));
}

#[test]
fn pkce_rejects_a_tampered_challenge() {
    assert!(!pkce_verifier_matches(
        VERIFIER,
        "RqIZl4LIgn8KxW9QO-nTnv7pf0CnNrksx9fF-CXP2F"
    ));
}

#[test]
fn pkce_rejects_an_empty_challenge() {
    assert!(!pkce_verifier_matches(VERIFIER, ""));
}

// LiteLLM hashes verifier.encode("ascii", "replace"), so each non-ASCII character counts as "?"
#[test]
fn pkce_hashes_non_ascii_characters_as_question_marks() {
    let replaced = "?".repeat(43);
    assert!(pkce_verifier_matches(
        &"\u{e9}".repeat(43),
        &challenge_of(&replaced)
    ));
    assert!(!pkce_verifier_matches(&"\u{e9}".repeat(43), CHALLENGE));
}

#[test]
fn pkce_never_matches_on_a_non_ascii_challenge() {
    assert!(!pkce_verifier_matches(VERIFIER, "\u{e9}"));
}

fn challenge_of(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[test]
fn no_master_key_is_a_server_error() {
    let error = signing_refusal(&SigningSetup::NoMasterKey).unwrap_err();
    assert_eq!(error, no_master_key_error());
    assert_eq!(error.status, 500);
    assert_eq!(error.error, OauthErrorCode::ServerError);
    assert_eq!(
        error.description,
        "the gateway has no master key configured"
    );
}

#[test]
fn invalid_signing_reports_one_fixed_description() {
    let setup = SigningSetup::Invalid(SessionSigningConfigError::Keys(SessionKeyError::EmptyKid));
    let error = signing_refusal(&setup).unwrap_err();
    assert_eq!(error, signing_config_error());
    assert_eq!(
        error.description,
        "the gateway session signing configuration is invalid"
    );
}

#[test]
fn ready_signing_hands_back_the_keys() {
    let setup = SigningSetup::Ready(keys());
    signing_refusal(&setup).unwrap();
}

#[rstest]
#[case(
    ReloadUserFailure::Unavailable,
    503,
    OauthErrorCode::TemporarilyUnavailable
)]
#[case(
    ReloadUserFailure::Faulted,
    503,
    OauthErrorCode::TemporarilyUnavailable
)]
#[case(ReloadUserFailure::Unresolvable, 500, OauthErrorCode::ServerError)]
#[case(ReloadUserFailure::NoActiveKey, 400, OauthErrorCode::InvalidGrant)]
fn reload_failures_map_to_oauth_errors(
    #[case] failure: ReloadUserFailure,
    #[case] status: u16,
    #[case] code: OauthErrorCode,
) {
    let error = reload_failure_error(failure);
    assert_eq!(error.status, status);
    assert_eq!(error.error, code);
}

#[test]
fn reload_database_failures_keep_their_descriptions() {
    assert_eq!(
        reload_failure_error(ReloadUserFailure::Unavailable).description,
        DB_UNAVAILABLE_DESCRIPTION
    );
    assert_eq!(
        reload_failure_error(ReloadUserFailure::Faulted).description,
        DB_FAULTED_DESCRIPTION
    );
    assert_eq!(
        reload_failure_error(ReloadUserFailure::Unresolvable).description,
        "the gateway is not configured to resolve users"
    );
    assert_eq!(
        reload_failure_error(ReloadUserFailure::NoActiveKey).description,
        "the user for this grant is no longer active"
    );
}

#[test]
fn mint_membership_failures_have_their_own_texts() {
    assert_eq!(
        mint_failure_error(ProxyCredentialMintFailure::NotAMember),
        oauth_error(
            400,
            OauthErrorCode::InvalidGrant,
            "the user is no longer a member of the team this grant was issued for"
        )
    );
    assert_eq!(
        mint_failure_error(ProxyCredentialMintFailure::TeamRequired),
        oauth_error(
            400,
            OauthErrorCode::InvalidGrant,
            "this user belongs to a team; sign in again and pick the team for this credential"
        )
    );
}

#[test]
fn mint_reload_failures_share_the_reload_text() {
    assert_eq!(
        mint_failure_error(ProxyCredentialMintFailure::Unavailable),
        reload_failure_error(ReloadUserFailure::Unavailable)
    );
    assert_eq!(
        mint_failure_error(ProxyCredentialMintFailure::Faulted),
        reload_failure_error(ReloadUserFailure::Faulted)
    );
    assert_eq!(
        mint_failure_error(ProxyCredentialMintFailure::Unresolvable),
        reload_failure_error(ReloadUserFailure::Unresolvable)
    );
    assert_eq!(
        mint_failure_error(ProxyCredentialMintFailure::NoActiveKey),
        reload_failure_error(ReloadUserFailure::NoActiveKey)
    );
}

#[rstest]
#[case(RefusalCode::InvalidRequest, 400, OauthErrorCode::InvalidRequest)]
#[case(
    RefusalCode::UnsupportedGrantType,
    400,
    OauthErrorCode::UnsupportedGrantType
)]
#[case(
    RefusalCode::TemporarilyUnavailable,
    503,
    OauthErrorCode::TemporarilyUnavailable
)]
fn subject_refusals_keep_the_idp_text(
    #[case] error: RefusalCode,
    #[case] status: u16,
    #[case] code: OauthErrorCode,
) {
    let refusal = SubjectTokenRefusal {
        error,
        description: "the IdP said no".to_owned(),
    };
    let mapped = subject_refusal_error(&refusal);
    assert_eq!(mapped.status, status);
    assert_eq!(mapped.error, code);
    assert_eq!(mapped.description, "the IdP said no");
}

#[test]
fn mint_and_client_refusals_have_fixed_texts() {
    assert_eq!(
        failed_to_mint_error().description,
        "failed to mint the session credential"
    );
    assert_eq!(
        unknown_client_error(),
        oauth_error(
            401,
            OauthErrorCode::InvalidClient,
            "unknown or malformed client_id"
        )
    );
}

#[test]
fn a_session_pair_serializes_in_wire_order() {
    let body = SessionPairBody {
        access_token: "llm_session_a".to_owned(),
        token_type: BEARER_TOKEN_TYPE,
        expires_in: 3_600,
        refresh_token: "llm_srefresh_r".to_owned(),
    };
    let text = serde_json::to_string(&body).unwrap();
    assert_eq!(
        text,
        r#"{"access_token":"llm_session_a","token_type":"Bearer","expires_in":3600,"refresh_token":"llm_srefresh_r"}"#
    );
}

#[test]
fn a_proxy_credential_keeps_a_null_team() {
    let body = ProxyCredentialBody {
        access_token: "sk-proxy".to_owned(),
        token_type: BEARER_TOKEN_TYPE,
        expires_in: 600,
        refresh_token: "llm_srefresh_r".to_owned(),
        user_id: "alice".to_owned(),
        team_id: None,
        issued_token_type: None,
    };
    let text = serde_json::to_string(&body).unwrap();
    assert_eq!(
        text,
        r#"{"access_token":"sk-proxy","token_type":"Bearer","expires_in":600,"refresh_token":"llm_srefresh_r","user_id":"alice","team_id":null}"#
    );
}

#[test]
fn a_proxy_credential_names_the_issued_type_when_set() {
    let body = ProxyCredentialBody {
        access_token: "sk-proxy".to_owned(),
        token_type: BEARER_TOKEN_TYPE,
        expires_in: 600,
        refresh_token: "llm_srefresh_r".to_owned(),
        user_id: "alice".to_owned(),
        team_id: Some("team-a".to_owned()),
        issued_token_type: Some(ACCESS_TOKEN_TOKEN_TYPE),
    };
    let text = serde_json::to_string(&body).unwrap();
    assert!(text.ends_with(
        r#""team_id":"team-a","issued_token_type":"urn:ietf:params:oauth:token-type:access_token"}"#
    ));
}

#[test]
fn a_session_pair_debug_redacts_both_tokens() {
    let pair = SessionPairBody {
        access_token: "llm_session_a".to_owned(),
        token_type: BEARER_TOKEN_TYPE,
        expires_in: 3_600,
        refresh_token: "llm_srefresh_r".to_owned(),
    };
    let text = format!("{pair:?}");
    assert!(text.contains("<redacted>"));
    assert!(!text.contains("llm_session_a"));
    assert!(!text.contains("llm_srefresh_r"));
}

#[test]
fn a_proxy_credential_debug_redacts_the_key_and_refresh_token() {
    let proxy = ProxyCredentialBody {
        access_token: "sk-proxy".to_owned(),
        token_type: BEARER_TOKEN_TYPE,
        expires_in: 600,
        refresh_token: "llm_srefresh_r".to_owned(),
        user_id: "alice".to_owned(),
        team_id: None,
        issued_token_type: None,
    };
    let text = format!("{proxy:?}");
    assert!(text.contains("<redacted>"));
    assert!(!text.contains("sk-proxy"));
    assert!(!text.contains("llm_srefresh_r"));
}

#[test]
fn revocation_renders_an_empty_object() {
    assert_eq!(
        serde_json::to_string(&RevokedBody::default()).unwrap(),
        "{}"
    );
}

#[test]
fn an_inactive_introspection_renders_only_active() {
    let body = IntrospectionBody {
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
    };
    assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"active":false}"#);
}

#[test]
fn an_active_introspection_serializes_in_wire_order() {
    let body = IntrospectionBody {
        active: true,
        iss: Some("litellm-mcp-gateway"),
        sub: Some("alice".to_owned()),
        client_id: Some("client-1".to_owned()),
        jti: Some("BwcHBwcHBwcHBwcHBwcHBw".to_owned()),
        iat: Some(1_700_000_000),
        exp: Some(1_700_003_600),
        kind: Some(SessionTokenKind::Session),
        token_type: Some(BEARER_TOKEN_TYPE),
        team_id: Some("team-a".to_owned()),
        resource_server_id: Some("srv-a".to_owned()),
        audience: Some(SessionAudience::ProxyApi),
    };
    let text = serde_json::to_string(&body).unwrap();
    assert_eq!(
        text,
        r#"{"active":true,"iss":"litellm-mcp-gateway","sub":"alice","client_id":"client-1","jti":"BwcHBwcHBwcHBwcHBwcHBw","iat":1700000000,"exp":1700003600,"kind":"session","token_type":"Bearer","team_id":"team-a","resource_server_id":"srv-a","audience":"proxy_api"}"#
    );
}

#[test]
fn json_replies_carry_the_no_cache_headers() {
    let reply = json_reply(200, TokenBody::Revoked(RevokedBody::default()));
    match reply {
        TokenReply::Json {
            status, headers, ..
        } => {
            assert_eq!(status, 200);
            assert_eq!(
                headers,
                vec![
                    ("Cache-Control".to_owned(), "no-store".to_owned()),
                    ("Pragma".to_owned(), "no-cache".to_owned()),
                ]
            );
        }
        TokenReply::Error(error) => panic!("expected a json reply, got {}", error.description),
    }
}
