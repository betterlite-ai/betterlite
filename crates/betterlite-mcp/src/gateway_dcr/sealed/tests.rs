#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher,
    StoredValue,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::gateway_dcr::contracts::{
    AUTH_CODE_DEBUG_KEY, CONNECT_FLOW_DEBUG_KEY, GATEWAY_AUTH_CODE_PREFIX,
    GATEWAY_DCR_CLIENT_ID_PREFIX,
};
use crate::gateway_dcr::models::{ConnectFlow, GatewayAuthCode, GatewayDcrClient};
use crate::gateway_dcr::sealed::{SealError, open_gateway_dcr_client, open_sealed, seal};
use crate::types::SessionAudience;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fuzz/corpus/gateway_dcr_open_sealed"
);

const CORPUS_SALT: &str = "b4674-corpus-salt";

#[derive(Serialize)]
struct ClientWithExtraField {
    redirect_uris: Vec<String>,
    iat: i64,
    extra: bool,
}

#[derive(Serialize)]
struct FlowWithoutOptionals {
    user_id: String,
    client_id: String,
    redirect_uri: String,
    state: String,
    code_challenge: String,
    jti: String,
    exp: i64,
}

fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(CORPUS_SALT)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn cipher_without_a_key() -> DbCipher {
    let mut cipher = cipher();
    cipher.settings.signing_key = None;
    cipher
}

fn client() -> GatewayDcrClient {
    GatewayDcrClient {
        redirect_uris: vec![
            "https://client.example/callback".to_owned(),
            "http://localhost:8080/callback".to_owned(),
        ],
        iat: 1_700_000_000,
    }
}

fn flow() -> ConnectFlow {
    ConnectFlow {
        user_id: "user-1".to_owned(),
        client_id: "llm_dcrc_client".to_owned(),
        redirect_uri: "https://client.example/callback".to_owned(),
        state: String::new(),
        code_challenge: "challenge-1".to_owned(),
        jti: "jti-1".to_owned(),
        exp: 1_700_000_600,
        resource_server_id: None,
        audience: None,
    }
}

fn auth_code() -> GatewayAuthCode {
    GatewayAuthCode {
        user_id: "user-1".to_owned(),
        client_id: "llm_dcrc_client".to_owned(),
        redirect_uri: "https://client.example/callback".to_owned(),
        code_challenge: "challenge-1".to_owned(),
        jti: "jti-1".to_owned(),
        iat: 1_700_000_000,
        exp: 1_700_000_120,
        resource_server_id: Some("srv-1".to_owned()),
        audience: Some(SessionAudience::ProxyApi),
        team_id: Some("team-1".to_owned()),
    }
}

fn plaintext(sealed: &str, prefix: &str, cipher: &DbCipher) -> String {
    let ciphertext = sealed.strip_prefix(prefix).unwrap();
    let stored = StoredValue::<()>::Text(SecretString::from(ciphertext.to_owned()));
    let opened = cipher
        .decrypt_value(
            stored,
            "b4674-test",
            DecryptPolicy {
                log: FailureLog::Debug,
                value: FailureValue::Nothing,
            },
        )
        .unwrap();
    let StoredValue::Text(text) = opened else {
        panic!("a sealed payload decrypts to text");
    };
    text.expose_secret().to_owned()
}

fn tampered(sealed: &str) -> String {
    let (head, tail) = sealed.split_at(sealed.len().div_euclid(2));
    let (changed, rest) = tail.split_at(1);
    let replacement = if changed == "A" { "B" } else { "A" };
    format!("{head}{replacement}{rest}")
}

fn corpus_file(name: &str) -> String {
    let path = Path::new(CORPUS).join(name);
    let bytes = fs::read(&path).unwrap();
    str::from_utf8(&bytes).unwrap().to_owned()
}

#[test]
fn sealed_client_id_opens_to_the_same_redirect_uris() {
    let sealed = seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client(), &cipher()).unwrap();
    assert!(sealed.starts_with(GATEWAY_DCR_CLIENT_ID_PREFIX));
    assert_eq!(
        open_gateway_dcr_client(&sealed, &cipher()).unwrap(),
        client()
    );
}

#[test]
fn sealing_twice_gives_a_fresh_ciphertext() {
    let first = seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client(), &cipher()).unwrap();
    let second = seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client(), &cipher()).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        open_gateway_dcr_client(&second, &cipher()).unwrap(),
        client()
    );
}

#[test]
fn open_rejects_a_tampered_ciphertext() {
    let sealed = seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client(), &cipher()).unwrap();
    assert!(open_gateway_dcr_client(&tampered(&sealed), &cipher()).is_none());
}

#[test]
fn open_rejects_a_truncated_value() {
    let sealed = seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client(), &cipher()).unwrap();
    let half = sealed.get(..sealed.len().div_euclid(2)).unwrap();
    assert!(open_gateway_dcr_client(half, &cipher()).is_none());
    let missing_last = sealed.get(..sealed.len().saturating_sub(1)).unwrap();
    assert!(open_gateway_dcr_client(missing_last, &cipher()).is_none());
}

#[test]
fn open_rejects_a_wrong_prefix() {
    let sealed = seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client(), &cipher()).unwrap();
    let ciphertext = sealed.strip_prefix(GATEWAY_DCR_CLIENT_ID_PREFIX).unwrap();
    let wrong = format!("{GATEWAY_AUTH_CODE_PREFIX}{ciphertext}");
    assert!(open_gateway_dcr_client(&wrong, &cipher()).is_none());
}

#[test]
fn open_rejects_a_payload_that_is_not_a_client_record() {
    let prefix = GATEWAY_DCR_CLIENT_ID_PREFIX;
    let array = seal(prefix, &[1_u32, 2_u32], &cipher()).unwrap();
    let text = seal(prefix, &"a string payload", &cipher()).unwrap();
    let flow = seal(prefix, &flow(), &cipher()).unwrap();
    let code = seal(prefix, &auth_code(), &cipher()).unwrap();
    assert!(open_gateway_dcr_client(&array, &cipher()).is_none());
    assert!(open_gateway_dcr_client(&text, &cipher()).is_none());
    assert!(open_gateway_dcr_client(&flow, &cipher()).is_none());
    assert!(open_gateway_dcr_client(&code, &cipher()).is_none());
}

#[test]
fn open_rejects_extra_fields_and_the_redirect_uri_count() {
    let prefix = GATEWAY_DCR_CLIENT_ID_PREFIX;
    let extra = ClientWithExtraField {
        redirect_uris: vec!["https://client.example/callback".to_owned()],
        iat: 1,
        extra: true,
    };
    let none = GatewayDcrClient {
        redirect_uris: Vec::new(),
        iat: 1,
    };
    let five = GatewayDcrClient {
        redirect_uris: vec!["https://a.example/1".to_owned(); 5],
        iat: 1,
    };
    let with_extra = seal(prefix, &extra, &cipher()).unwrap();
    let with_none = seal(prefix, &none, &cipher()).unwrap();
    let with_five = seal(prefix, &five, &cipher()).unwrap();
    assert!(open_gateway_dcr_client(&with_extra, &cipher()).is_none());
    assert!(open_gateway_dcr_client(&with_none, &cipher()).is_none());
    assert!(open_gateway_dcr_client(&with_five, &cipher()).is_none());
}

#[test]
fn open_rejects_empty_required_fields() {
    let mut empty_user = flow();
    empty_user.user_id = String::new();
    let sealed_flow = seal("", &empty_user, &cipher()).unwrap();
    assert!(
        open_sealed::<ConnectFlow>(&sealed_flow, "", &cipher(), CONNECT_FLOW_DEBUG_KEY).is_none()
    );

    let mut empty_jti = auth_code();
    empty_jti.jti = String::new();
    let sealed_code = seal(GATEWAY_AUTH_CODE_PREFIX, &empty_jti, &cipher()).unwrap();
    assert!(
        open_sealed::<GatewayAuthCode>(
            &sealed_code,
            GATEWAY_AUTH_CODE_PREFIX,
            &cipher(),
            AUTH_CODE_DEBUG_KEY
        )
        .is_none()
    );
}

#[rstest]
#[case::empty("")]
#[case::prefix_only("llm_dcrc_")]
#[case::nul_bytes("llm_dcrc_\u{0}\u{1}")]
#[case::not_base64("not base64 !")]
#[case::json_braces("llm_dcrc_{}")]
#[case::base64_then_junk("llm_dcrc_AAAA")]
fn open_is_total_on_hostile_text(#[case] text: &str) {
    assert!(open_gateway_dcr_client(text, &cipher()).is_none());
}

#[test]
fn open_with_an_empty_prefix_reads_the_connect_flow() {
    let sealed = seal("", &flow(), &cipher()).unwrap();
    let opened =
        open_sealed::<ConnectFlow>(&sealed, "", &cipher(), CONNECT_FLOW_DEBUG_KEY).unwrap();
    assert_eq!(opened, flow());
}

#[test]
fn sealed_optional_fields_are_absent_from_the_json() {
    let sealed_flow = seal("", &flow(), &cipher()).unwrap();
    let text = plaintext(&sealed_flow, "", &cipher());
    assert!(!text.contains("resource_server_id"));
    assert!(!text.contains("audience"));
    assert!(text.contains(r#""state":"""#));

    let sealed_code = seal(GATEWAY_AUTH_CODE_PREFIX, &auth_code(), &cipher()).unwrap();
    let code_text = plaintext(&sealed_code, GATEWAY_AUTH_CODE_PREFIX, &cipher());
    assert!(code_text.contains(r#""resource_server_id":"srv-1""#));
    assert!(code_text.contains(r#""audience":"proxy_api""#));
    assert!(code_text.contains(r#""team_id":"team-1""#));

    let mut bare = auth_code();
    bare.resource_server_id = None;
    bare.audience = None;
    bare.team_id = None;
    let sealed_bare = seal(GATEWAY_AUTH_CODE_PREFIX, &bare, &cipher()).unwrap();
    let bare_text = plaintext(&sealed_bare, GATEWAY_AUTH_CODE_PREFIX, &cipher());
    assert!(!bare_text.contains("resource_server_id"));
    assert!(!bare_text.contains("audience"));
    assert!(!bare_text.contains("team_id"));
}

#[test]
fn a_blob_without_the_optional_fields_opens_with_them_none() {
    let bare = FlowWithoutOptionals {
        user_id: "user-1".to_owned(),
        client_id: "llm_dcrc_client".to_owned(),
        redirect_uri: "https://client.example/callback".to_owned(),
        state: "state-1".to_owned(),
        code_challenge: "challenge-1".to_owned(),
        jti: "jti-1".to_owned(),
        exp: 1_700_000_600,
    };
    let sealed = seal("", &bare, &cipher()).unwrap();
    let opened =
        open_sealed::<ConnectFlow>(&sealed, "", &cipher(), CONNECT_FLOW_DEBUG_KEY).unwrap();
    assert_eq!(opened.state, "state-1");
    assert!(opened.resource_server_id.is_none());
    assert!(opened.audience.is_none());
}

#[test]
fn seal_reports_a_cipher_that_refuses() {
    let refusal = seal(
        GATEWAY_DCR_CLIENT_ID_PREFIX,
        &client(),
        &cipher_without_a_key(),
    );
    assert!(matches!(refusal, Err(SealError::Encrypt(_))));
}

#[rstest]
fn replays_the_gateway_dcr_open_sealed_corpus(
    #[files("fuzz/corpus/gateway_dcr_open_sealed/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let _opened = open_gateway_dcr_client(text, &cipher());
}

#[test]
fn opens_only_the_client_seed_and_refuses_the_hostile_ones() {
    let opened = open_gateway_dcr_client(&corpus_file("client"), &cipher()).unwrap();
    assert_eq!(
        opened.redirect_uris,
        vec!["https://client.example/callback".to_owned()]
    );
    assert_eq!(opened.iat, 1_700_000_000);
    assert!(open_gateway_dcr_client(&corpus_file("auth-code-as-client"), &cipher()).is_none());
    assert!(open_gateway_dcr_client(&corpus_file("extra-field-as-client"), &cipher()).is_none());
    assert!(open_gateway_dcr_client(&corpus_file("truncated"), &cipher()).is_none());
    assert!(open_gateway_dcr_client(&corpus_file("wrong-prefix"), &cipher()).is_none());
    assert!(open_gateway_dcr_client(&corpus_file("raw-text"), &cipher()).is_none());
    assert!(open_gateway_dcr_client(&corpus_file("prefix-only"), &cipher()).is_none());
    assert!(open_gateway_dcr_client(&corpus_file("empty"), &cipher()).is_none());
}
