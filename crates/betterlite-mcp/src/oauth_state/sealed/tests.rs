use std::fs;
use std::str;

use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use secrecy::ExposeSecret as _;

use crate::oauth_state::SealCipher;
use crate::oauth_state::test_support::{FieldBoundCipher, PlainCipher, RefusingCipher};

use super::{
    BRIDGE_CODE_PREFIX, BridgeCodePrefix, PASSTHROUGH_CODE_PREFIX, PassthroughClient, RedeemError,
    open_bridge_code, open_passthrough_code, redeem_passthrough_code, seal_bridge_code,
    seal_passthrough_code,
};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/mcp_sealed_code");
const MARKER: &str = "fuzz:";

const BRIDGE_JSON: &str = r#"{"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":"nonce-1","upstream_code":"up-1"}"#;
const BRIDGE_NULL_NONCE_JSON: &str = r#"{"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":null,"upstream_code":"up-1"}"#;
const PASSTHROUGH_JSON: &str = r#"{"client_id":"client-1","client_secret":"secret-1","mcp_server_id":"srv-1","token_endpoint_auth_method":"client_secret_post","upstream_code":"up-1"}"#;
const PASSTHROUGH_NULL_JSON: &str = r#"{"client_id":"client-1","client_secret":null,"mcp_server_id":"srv-1","token_endpoint_auth_method":null,"upstream_code":"up-1"}"#;

struct ReplayCipher;

impl SealCipher for ReplayCipher {
    fn encrypt(&self, plain: &str) -> String {
        format!("{MARKER}{plain}")
    }

    fn decrypt(&self, value: &str, _field: &str) -> Option<String> {
        value.strip_prefix(MARKER).map(str::to_owned)
    }
}

fn bridge_code(json: &str) -> String {
    format!("{BRIDGE_CODE_PREFIX}{json}")
}

fn passthrough_code(json: &str) -> String {
    format!("{PASSTHROUGH_CODE_PREFIX}{json}")
}

fn any_bridge_opens(jsons: &[&str]) -> bool {
    jsons
        .iter()
        .any(|json| open_bridge_code(&bridge_code(json), &PlainCipher).is_some())
}

fn any_passthrough_opens(jsons: &[&str]) -> bool {
    jsons
        .iter()
        .any(|json| open_passthrough_code(&passthrough_code(json), &PlainCipher).is_some())
}

fn full_client() -> PassthroughClient<'static> {
    PassthroughClient {
        client_id: "client-1",
        client_secret: Some("secret-1"),
        token_endpoint_auth_method: Some(MCPTokenEndpointAuthMethod::ClientSecretPost),
    }
}

fn sealed_passthrough() -> String {
    seal_passthrough_code("up-1", &full_client(), "srv-1", &PlainCipher)
}

#[test]
fn is_bridge_code_matches_only_the_bridge_prefix() {
    assert!(bridge_code("payload").is_bridge_code());
    assert!(!passthrough_code("payload").is_bridge_code());
    assert!(!"upstream-code".is_bridge_code());
}

#[test]
fn seal_bridge_code_writes_the_sorted_payload() {
    let sealed = seal_bridge_code("up-1", "user-1", "srv-1", Some("nonce-1"), &PlainCipher);
    assert_eq!(sealed, bridge_code(BRIDGE_JSON));
}

#[test]
fn seal_bridge_code_keeps_a_null_nonce() {
    let sealed = seal_bridge_code("up-1", "user-1", "srv-1", None, &PlainCipher);
    assert_eq!(sealed, bridge_code(BRIDGE_NULL_NONCE_JSON));
}

#[test]
fn open_bridge_code_reads_every_field() {
    let sealed = seal_bridge_code("up-1", "user-1", "srv-1", Some("nonce-1"), &PlainCipher);
    let opened = open_bridge_code(&sealed, &PlainCipher).unwrap();
    assert_eq!(opened.upstream_code, "up-1");
    assert_eq!(opened.litellm_user_id, "user-1");
    assert_eq!(opened.mcp_server_id, "srv-1");
    assert_eq!(opened.oauth_nonce.as_deref(), Some("nonce-1"));
}

#[test]
fn open_bridge_code_reads_an_absent_nonce_as_null() {
    let opened = open_bridge_code(&bridge_code(BRIDGE_NULL_NONCE_JSON), &PlainCipher).unwrap();
    assert!(opened.oauth_nonce.is_none());
}

#[test]
fn open_bridge_code_rejects_a_raw_code() {
    assert!(open_bridge_code("raw-upstream-code", &PlainCipher).is_none());
    assert!(open_bridge_code(&passthrough_code(BRIDGE_JSON), &PlainCipher).is_none());
}

#[test]
fn open_bridge_code_rejects_a_failed_decrypt() {
    assert!(open_bridge_code(&bridge_code(BRIDGE_JSON), &RefusingCipher).is_none());
}

#[test]
fn open_bridge_code_rejects_bad_json() {
    assert!(open_bridge_code(&bridge_code("not json"), &PlainCipher).is_none());
    assert!(open_bridge_code(&bridge_code("[]"), &PlainCipher).is_none());
}

#[test]
fn open_bridge_code_rejects_a_missing_required_field() {
    assert!(!any_bridge_opens(&[
        r#"{"mcp_server_id":"srv-1","oauth_nonce":null,"upstream_code":"up-1"}"#,
        r#"{"litellm_user_id":"user-1","oauth_nonce":null,"upstream_code":"up-1"}"#,
        r#"{"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":null}"#,
    ]));
}

#[test]
fn open_bridge_code_rejects_an_empty_required_field() {
    assert!(!any_bridge_opens(&[
        r#"{"litellm_user_id":"","mcp_server_id":"srv-1","oauth_nonce":null,"upstream_code":"up-1"}"#,
        r#"{"litellm_user_id":"user-1","mcp_server_id":"","oauth_nonce":null,"upstream_code":"up-1"}"#,
        r#"{"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":null,"upstream_code":""}"#,
    ]));
}

#[test]
fn open_bridge_code_rejects_a_wrong_field_type() {
    assert!(!any_bridge_opens(&[
        r#"{"litellm_user_id":5,"mcp_server_id":"srv-1","oauth_nonce":null,"upstream_code":"up-1"}"#,
        r#"{"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":5,"upstream_code":"up-1"}"#,
    ]));
}

#[test]
fn open_bridge_code_keeps_an_empty_nonce() {
    let json = r#"{"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":"","upstream_code":"up-1"}"#;
    let opened = open_bridge_code(&bridge_code(json), &PlainCipher).unwrap();
    assert_eq!(opened.oauth_nonce.as_deref(), Some(""));
}

#[test]
fn open_bridge_code_ignores_unknown_keys() {
    let json = r#"{"extra":true,"litellm_user_id":"user-1","mcp_server_id":"srv-1","oauth_nonce":null,"upstream_code":"up-1"}"#;
    let opened = open_bridge_code(&bridge_code(json), &PlainCipher).unwrap();
    assert_eq!(opened.upstream_code, "up-1");
}

#[test]
fn open_bridge_code_passes_the_bridge_label() {
    let opened = open_bridge_code(
        &bridge_code(BRIDGE_JSON),
        &FieldBoundCipher {
            field: "bridge_authorization_code",
        },
    );
    assert!(opened.is_some());
    let refused = open_bridge_code(
        &bridge_code(BRIDGE_JSON),
        &FieldBoundCipher {
            field: "passthrough_authorization_code",
        },
    );
    assert!(refused.is_none());
}

#[test]
fn seal_passthrough_code_writes_the_sorted_payload() {
    let sealed = seal_passthrough_code("up-1", &full_client(), "srv-1", &PlainCipher);
    assert_eq!(sealed, passthrough_code(PASSTHROUGH_JSON));
}

#[test]
fn seal_passthrough_code_keeps_nulls() {
    let client = PassthroughClient {
        client_id: "client-1",
        client_secret: None,
        token_endpoint_auth_method: None,
    };
    let sealed = seal_passthrough_code("up-1", &client, "srv-1", &PlainCipher);
    assert_eq!(sealed, passthrough_code(PASSTHROUGH_NULL_JSON));
}

#[test]
fn open_passthrough_code_reads_every_field() {
    let opened = open_passthrough_code(&sealed_passthrough(), &PlainCipher).unwrap();
    assert_eq!(opened.upstream_code, "up-1");
    assert_eq!(opened.client_id, "client-1");
    assert_eq!(opened.mcp_server_id, "srv-1");
    assert_eq!(
        opened
            .client_secret
            .as_ref()
            .map(|stored| stored.0.expose_secret().to_owned()),
        Some("secret-1".to_owned())
    );
    assert_eq!(
        opened.token_endpoint_auth_method,
        Some(MCPTokenEndpointAuthMethod::ClientSecretPost)
    );
}

#[test]
fn open_passthrough_code_reads_nulls() {
    let opened =
        open_passthrough_code(&passthrough_code(PASSTHROUGH_NULL_JSON), &PlainCipher).unwrap();
    assert!(opened.client_secret.is_none());
    assert!(opened.token_endpoint_auth_method.is_none());
}

#[test]
fn open_passthrough_code_rejects_a_raw_code() {
    assert!(open_passthrough_code("raw-upstream-code", &PlainCipher).is_none());
    assert!(open_passthrough_code(&bridge_code(PASSTHROUGH_JSON), &PlainCipher).is_none());
}

#[test]
fn open_passthrough_code_rejects_a_failed_decrypt() {
    assert!(open_passthrough_code(&passthrough_code(PASSTHROUGH_JSON), &RefusingCipher).is_none());
}

#[test]
fn open_passthrough_code_rejects_bad_json() {
    assert!(open_passthrough_code(&passthrough_code("{"), &PlainCipher).is_none());
}

#[test]
fn open_passthrough_code_rejects_an_empty_required_field() {
    assert!(!any_passthrough_opens(&[
        r#"{"client_id":"","client_secret":null,"mcp_server_id":"srv-1","token_endpoint_auth_method":null,"upstream_code":"up-1"}"#,
        r#"{"client_id":"client-1","client_secret":null,"mcp_server_id":"","token_endpoint_auth_method":null,"upstream_code":"up-1"}"#,
        r#"{"client_id":"client-1","client_secret":null,"mcp_server_id":"srv-1","token_endpoint_auth_method":null,"upstream_code":""}"#,
    ]));
}

#[test]
fn open_passthrough_code_rejects_an_unknown_auth_method() {
    assert!(!any_passthrough_opens(&[
        r#"{"client_id":"client-1","client_secret":null,"mcp_server_id":"srv-1","token_endpoint_auth_method":"private_key_jwt","upstream_code":"up-1"}"#,
        r#"{"client_id":"client-1","client_secret":null,"mcp_server_id":"srv-1","token_endpoint_auth_method":5,"upstream_code":"up-1"}"#,
        r#"{"client_id":"client-1","client_secret":5,"mcp_server_id":"srv-1","token_endpoint_auth_method":null,"upstream_code":"up-1"}"#,
    ]));
}

#[test]
fn open_passthrough_code_keeps_an_empty_client_secret() {
    let json = r#"{"client_id":"client-1","client_secret":"","mcp_server_id":"srv-1","token_endpoint_auth_method":null,"upstream_code":"up-1"}"#;
    let opened = open_passthrough_code(&passthrough_code(json), &PlainCipher).unwrap();
    assert_eq!(
        opened
            .client_secret
            .as_ref()
            .map(|stored| stored.0.expose_secret().to_owned()),
        Some(String::new())
    );
}

#[test]
fn open_passthrough_code_ignores_unknown_keys() {
    let json = r#"{"client_id":"client-1","client_secret":null,"extra":1,"mcp_server_id":"srv-1","token_endpoint_auth_method":null,"upstream_code":"up-1"}"#;
    assert!(open_passthrough_code(&passthrough_code(json), &PlainCipher).is_some());
}

#[test]
fn open_passthrough_code_passes_the_passthrough_label() {
    let opened = open_passthrough_code(
        &passthrough_code(PASSTHROUGH_JSON),
        &FieldBoundCipher {
            field: "passthrough_authorization_code",
        },
    );
    assert!(opened.is_some());
    let refused = open_passthrough_code(
        &passthrough_code(PASSTHROUGH_JSON),
        &FieldBoundCipher {
            field: "bridge_authorization_code",
        },
    );
    assert!(refused.is_none());
}

#[test]
fn redeem_passthrough_code_returns_none_without_a_code() {
    assert!(
        redeem_passthrough_code(None, "srv-1", Some("verifier"), &PlainCipher)
            .unwrap()
            .is_none()
    );
    assert!(
        redeem_passthrough_code(Some(""), "srv-1", None, &PlainCipher)
            .unwrap()
            .is_none()
    );
}

#[test]
fn redeem_passthrough_code_returns_none_for_a_raw_code() {
    assert!(
        redeem_passthrough_code(Some("raw-upstream-code"), "srv-1", None, &PlainCipher)
            .unwrap()
            .is_none()
    );
}

#[test]
fn redeem_passthrough_code_rejects_another_server() {
    let sealed = sealed_passthrough();
    let failure = redeem_passthrough_code(
        Some(sealed.as_str()),
        "srv-2",
        Some("verifier"),
        &PlainCipher,
    )
    .unwrap_err();
    assert_eq!(failure, RedeemError::WrongServer);
    assert_eq!(
        failure.to_string(),
        "Authorization code was issued for a different MCP server"
    );
}

#[test]
fn redeem_passthrough_code_requires_a_verifier() {
    let sealed = sealed_passthrough();
    let missing =
        redeem_passthrough_code(Some(sealed.as_str()), "srv-1", None, &PlainCipher).unwrap_err();
    assert_eq!(missing, RedeemError::VerifierRequired);
    assert_eq!(
        missing.to_string(),
        "code_verifier is required to redeem this authorization code"
    );
    let empty = redeem_passthrough_code(Some(sealed.as_str()), "srv-1", Some(""), &PlainCipher)
        .unwrap_err();
    assert_eq!(empty, RedeemError::VerifierRequired);
}

#[test]
fn redeem_passthrough_code_checks_the_server_before_the_verifier() {
    let sealed = sealed_passthrough();
    let failure =
        redeem_passthrough_code(Some(sealed.as_str()), "srv-2", None, &PlainCipher).unwrap_err();
    assert_eq!(failure, RedeemError::WrongServer);
}

#[test]
fn redeem_passthrough_code_returns_the_sealed_value() {
    let sealed = sealed_passthrough();
    let opened = redeem_passthrough_code(
        Some(sealed.as_str()),
        "srv-1",
        Some("verifier"),
        &PlainCipher,
    )
    .unwrap()
    .unwrap();
    assert_eq!(opened.upstream_code, "up-1");
    assert_eq!(opened.client_id, "client-1");
}

fn replay_corpus() -> (bool, bool) {
    let mut bridge_opened = false;
    let mut passthrough_opened = false;
    for entry in fs::read_dir(CORPUS).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = fs::read(&path).unwrap();
            let text = str::from_utf8(&bytes).unwrap();
            if open_bridge_code(text, &ReplayCipher).is_some() {
                bridge_opened = true;
            }
            if open_passthrough_code(text, &ReplayCipher).is_some() {
                passthrough_opened = true;
            }
        }
    }
    (bridge_opened, passthrough_opened)
}

#[test]
fn replays_sealed_corpus() {
    let (bridge_opened, passthrough_opened) = replay_corpus();
    assert!(bridge_opened, "no seed opens as a bridge code");
    assert!(passthrough_opened, "no seed opens as a passthrough code");
}
