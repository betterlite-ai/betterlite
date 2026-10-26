use crate::errors::SessionOpenError::{BadSignature, Expired, Malformed, NotASessionToken};
use crate::errors::{SessionKeyError, SessionOpenError};
use crate::session_tokens::{
    SessionPrefix as _, mint_session_token as mint_token, open_session_token as open_token,
};
use crate::types::SessionTokenKind::{Refresh, Session};
use crate::types::{
    AsymmetricSessionKeys, OpenedSessionToken, SessionKeys, SessionPrincipal,
    SessionRotatedPublicKey, SessionSigningKeys, SessionTokenKind,
};
use aws_lc_rs::{hmac, rand};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, TimeDelta, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::{fs, path::Path};

const SECRET: &str = "0123456789abcdef0123456789abcdef";
const OTHER_SECRET: &str = "fedcba9876543210fedcba9876543210";
const PRIVATE: &str = include_str!("../../fixtures/session-private.pem");
const PKCS1: &str = include_str!("../../fixtures/session-pkcs1.pem");
const PUBLIC: &str = include_str!("../../fixtures/session-public.pem");
const PREVIOUS: &str = include_str!("../../fixtures/session-previous.pem");
const PREVIOUS_PUBLIC: &str = include_str!("../../fixtures/session-previous-public.pem");
const WEAK: &str = include_str!("../../fixtures/session-weak.pem");
const WEAK_PUBLIC: &str = include_str!("../../fixtures/session-weak-public.pem");
const ENTROPY: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const NOW: i64 = 1_767_225_600;
const HS256: &str = r#"{"alg":"HS256"}"#;
const CLAIMS: &str = r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":9999999999,"jti":"j","kind":"session","user_id":"u","client_id":"c","team_id":null}"#;
// LiteLLM 1.105.1 mints this for principal() at NOW with jti bytes 0..16, as the oracle case symmetric-hs256-access shows
const LITELLM_HS256: &str = "llm_session_eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJsaXRlbGxtLW1jcC1nYXRld2F5IiwiaWF0IjoxNzY3MjI1NjAwLCJleHAiOjE3NjcyMjkyMDAsImp0aSI6IkFBRUNBd1FGQmdjSUNRb0xEQTBPRHciLCJraW5kIjoic2Vzc2lvbiIsInVzZXJfaWQiOiJcdTAwZmNzZXItMSIsImNsaWVudF9pZCI6ImNsaWVudC0xIiwicmVzb3VyY2Vfc2VydmVyX2lkIjoic2VydmVyLTEiLCJhdWRpZW5jZSI6InByb3h5X2FwaSIsInRlYW1faWQiOiJ0ZWFtLTEifQ.cDBqeejUJNbkdpbcKXkSYFte9nqUrT1rQ4QajEW9w4A";

type Opened = Result<(), SessionOpenError>;
type RsaKeys = Result<AsymmetricSessionKeys, SessionKeyError>;

fn asymmetric(pem: &str, kid: &str, previous: Vec<SessionRotatedPublicKey>) -> RsaKeys {
    AsymmetricSessionKeys::new(&SecretString::from(pem), kid.to_owned(), previous)
}

fn rotated(kid: &str, pem: &str) -> Result<Vec<SessionRotatedPublicKey>, SessionKeyError> {
    SessionRotatedPublicKey::new(kid.to_owned(), pem).map(|key| vec![key])
}

fn hs256(secret: &str) -> SessionSigningKeys {
    SessionSigningKeys::Symmetric(SessionKeys::new(SecretString::from(secret)).unwrap())
}

fn rs256(pem: &str, kid: &str) -> SessionSigningKeys {
    SessionSigningKeys::Asymmetric(asymmetric(pem, kid, Vec::new()).unwrap())
}

fn rotation() -> SessionSigningKeys {
    let previous = rotated("previous", PREVIOUS_PUBLIC).unwrap();
    SessionSigningKeys::Asymmetric(asymmetric(PKCS1, "current", previous).unwrap())
}

fn principal() -> SessionPrincipal {
    serde_json::from_str(include_str!("../../fixtures/session-principal.json")).unwrap()
}

fn at(seconds_later: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(NOW.checked_add(seconds_later).unwrap(), 0).unwrap()
}

fn mint(keys: &SessionSigningKeys, kind: SessionTokenKind) -> String {
    let minted = mint_token(&principal(), kind, keys, at(0), || Ok(ENTROPY)).unwrap();
    minted.token.expose_secret().to_owned()
}

fn forge(header: &str, payload: &str) -> String {
    let header_segment = URL_SAFE_NO_PAD.encode(header);
    let encoded = format!("{header_segment}.{}", URL_SAFE_NO_PAD.encode(payload));
    let key = hmac::Key::new(hmac::HMAC_SHA256, SECRET.as_bytes());
    let tag = hmac::sign(&key, encoded.as_bytes());
    format!("llm_session_{encoded}.{}", URL_SAFE_NO_PAD.encode(tag))
}

#[rstest]
#[case::hs256_access((hs256(SECRET), Session), (hs256(SECRET), Session), 0, Ok(3_600))]
#[case::hs256_refresh((hs256(SECRET), Refresh), (hs256(SECRET), Refresh), 1_209_599, Ok(1_209_600))]
#[case::rs256_access((rs256(PRIVATE, "current"), Session), (rs256(PRIVATE, "current"), Session), 0, Ok(3_600))]
#[case::rs256_refresh((rs256(PRIVATE, "current"), Refresh), (rs256(PRIVATE, "current"), Refresh), 0, Ok(1_209_600))]
#[case::rotation_current_pkcs1((rs256(PRIVATE, "current"), Session), (rotation(), Session), 0, Ok(3_600))]
#[case::rotation_previous((rs256(PREVIOUS, "previous"), Session), (rotation(), Session), 0, Ok(3_600))]
#[case::unknown_kid((rs256(PREVIOUS, "previous"), Session), (rs256(PRIVATE, "current"), Session), 0, Err(BadSignature))]
#[case::last_valid_second((hs256(SECRET), Session), (hs256(SECRET), Session), 3_599, Ok(3_600))]
#[case::expired_access((hs256(SECRET), Session), (hs256(SECRET), Session), 3_600, Err(Expired))]
#[case::expired_refresh((rs256(PRIVATE, "current"), Refresh), (rs256(PRIVATE, "current"), Refresh), 1_209_600, Err(Expired))]
#[case::bad_signature_hs256((hs256(SECRET), Session), (hs256(OTHER_SECRET), Session), 0, Err(BadSignature))]
#[case::bad_signature_rs256((rs256(PREVIOUS, "current"), Session), (rs256(PRIVATE, "current"), Session), 0, Err(BadSignature))]
#[case::access_opened_as_refresh((hs256(SECRET), Session), (hs256(SECRET), Refresh), 0, Err(NotASessionToken))]
#[case::hs256_token_under_rs256((hs256(SECRET), Session), (rs256(PRIVATE, "current"), Session), 0, Err(BadSignature))]
#[case::rs256_token_under_hs256((rs256(PRIVATE, "current"), Session), (hs256(SECRET), Session), 0, Err(Malformed))]
fn opens_what_it_mints(
    #[case] minted: (SessionSigningKeys, SessionTokenKind),
    #[case] opened_as: (SessionSigningKeys, SessionTokenKind),
    #[case] seconds_later: i64,
    #[case] lifetime: Result<i64, SessionOpenError>,
) {
    let token = mint(&minted.0, minted.1);
    let opened = open_token(&token, opened_as.1, &opened_as.0, at(seconds_later));
    let expected = lifetime.map(|seconds| OpenedSessionToken {
        principal: principal(),
        jti: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
        kind: minted.1,
        iat: NOW,
        exp: NOW.checked_add(seconds).unwrap(),
    });
    assert_eq!(opened, expected);
}

#[rstest]
#[case::control(HS256, CLAIMS, Ok(()))]
#[case::wrong_issuer(HS256, r#"{"iss":"elsewhere","iat":1,"exp":9999999999,"jti":"j","kind":"session","user_id":"u","client_id":"c"}"#, Err(Malformed))]
#[case::missing_exp(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"jti":"j","kind":"session","user_id":"u","client_id":"c"}"#, Err(Malformed))]
#[case::string_exp(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":"9999999999","jti":"j","kind":"session","user_id":"u","client_id":"c"}"#, Err(Malformed))]
#[case::float_iat(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1.0,"exp":9999999999,"jti":"j","kind":"session","user_id":"u","client_id":"c"}"#, Err(Malformed))]
#[case::extra_claim(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":9999999999,"jti":"j","kind":"session","user_id":"u","client_id":"c","sub":"u"}"#, Err(Malformed))]
#[case::empty_user_id(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":9999999999,"jti":"j","kind":"session","user_id":"","client_id":"c"}"#, Err(Malformed))]
#[case::empty_jti(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":9999999999,"jti":"","kind":"session","user_id":"u","client_id":"c"}"#, Err(Malformed))]
#[case::refresh_kind_reprefixed(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":9999999999,"jti":"j","kind":"session_refresh","user_id":"u","client_id":"c"}"#, Err(Malformed))]
#[case::unknown_audience(HS256, r#"{"iss":"litellm-mcp-gateway","iat":1,"exp":9999999999,"jti":"j","kind":"session","user_id":"u","client_id":"c","audience":"mcp"}"#, Err(Malformed))]
#[case::claims_not_an_object(HS256, "[]", Err(Malformed))]
#[case::alg_none(r#"{"alg":"none"}"#, "{}", Err(Malformed))]
#[case::numeric_kid(r#"{"alg":"HS256","kid":5}"#, "{}", Err(Malformed))]
fn checks_signed_claims(#[case] header: &str, #[case] payload: &str, #[case] expected: Opened) {
    let opened = open_token(&forge(header, payload), Session, &hs256(SECRET), at(0));
    assert_eq!(opened.map(drop), expected);
}

#[rstest]
#[case::bad_crit(r#""crit":"b64""#, false)]
#[case::null_crit(r#""crit":null"#, false)]
#[case::empty_crit(r#""crit":[]"#, false)]
#[case::non_string_crit(r#""crit":[1]"#, false)]
#[case::unsupported_crit(r#""crit":["x"],"x":true"#, false)]
#[case::missing_critical_header(r#""crit":["b64"]"#, false)]
#[case::b64_false(r#""b64":false"#, false)]
#[case::critical_b64_false(r#""crit":["b64"],"b64":false"#, false)]
#[case::supported_crit(r#""crit":["b64"],"b64":true"#, true)]
#[case::null_b64(r#""crit":["b64"],"b64":null"#, true)]
#[case::non_boolean_b64(r#""crit":["b64"],"b64":0"#, true)]
fn validates_header_extensions(
    #[case] extension: &str,
    #[case] valid: bool,
    #[values((hs256(SECRET), "HS256", Ok(())), (hs256(OTHER_SECRET), "HS256", Err(BadSignature)), (rs256(PRIVATE, "current"), "RS256", Err(BadSignature)))]
    signing: (SessionSigningKeys, &str, Opened),
) {
    let (keys, algorithm, signed) = signing;
    let header = format!(r#"{{"alg":"{algorithm}","kid":"unknown",{extension}}}"#);
    let expected = if valid { signed } else { Err(Malformed) };
    let opened = open_token(&forge(&header, CLAIMS), Session, &keys, at(0));
    assert_eq!(opened.map(drop), expected);
}

#[test]
fn preserves_rsa_key_selection_for_empty_detached_payload() {
    let header =
        URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","kid":"unknown","crit":["b64"],"b64":false}"#);
    let candidate = format!("llm_session_{header}..AA");
    let opened = open_token(&candidate, Session, &rs256(PRIVATE, "current"), at(0));
    assert_eq!(opened, Err(BadSignature));
}

#[rstest]
#[case::empty(String::new(), NotASessionToken)]
#[case::refresh_prefix("llm_srefresh_a.b.c".to_owned(), NotASessionToken)]
#[case::not_a_jwt("llm_session_not-a-jwt".to_owned(), Malformed)]
#[case::invalid_base64("llm_session_!!!.e30.e30".to_owned(), Malformed)]
#[case::padded_signature(format!("{LITELLM_HS256}="), Malformed)]
#[case::four_segments(format!("{LITELLM_HS256}.e30"), Malformed)]
#[case::one_byte_over(format!("llm_session_{}", "x".repeat(4_085)), Malformed)]
#[case::multibyte_over(format!("llm_session_{}", "\u{e9}".repeat(2_043)), Malformed)]
fn rejects_hostile_candidates(#[case] candidate: String, #[case] expected: SessionOpenError) {
    let opened = open_token(&candidate, Session, &hs256(SECRET), at(0));
    assert_eq!(opened, Err(expected));
}

#[test]
fn replays_the_session_token_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/session_token");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .filter_map(|entry| String::from_utf8(fs::read(entry.unwrap().path()).unwrap()).ok())
        .collect();
    let keys = [hs256(SECRET), rs256(PRIVATE, "current")];
    let opened = seeds
        .iter()
        .filter(|seed| {
            keys.iter()
                .any(|signing| open_token(seed, Session, signing, at(0)).is_ok())
        })
        .count();
    assert_eq!((seeds.len() > 100, opened), (true, 1));
}

#[test]
fn matches_litellm_signature_byte_for_byte() {
    assert_eq!(mint(&hs256(SECRET), Session), LITELLM_HS256);
}

#[rstest]
#[case::oversized_client_id("x".repeat(4_096), "TooLarge { size_bytes: 5853, max_bytes: 4096 }")]
#[case::empty_client_id(String::new(), "EmptyIdentity")]
fn refuses_to_mint(#[case] client_id: String, #[case] expected: &str) {
    let mut identity = principal();
    identity.client_id = client_id;
    let minted = mint_token(&identity, Session, &hs256(SECRET), at(0), || Ok(ENTROPY));
    assert_eq!(format!("{:?}", minted.unwrap_err()), expected);
}

#[test]
fn keeps_sub_second_expiry_and_fresh_jti() {
    let now = DateTime::from_timestamp(NOW, 500_000_000).unwrap();
    let keys = hs256(SECRET);
    let random = || rand::generate(&rand::SystemRandom::new()).map(rand::Random::expose);
    let first = mint_token(&principal(), Session, &keys, now, random).unwrap();
    let second = mint_token(&principal(), Session, &keys, now, random).unwrap();
    assert_eq!(first.expires_at, now + TimeDelta::seconds(3_600));
    assert_ne!(first.token.expose_secret(), second.token.expose_secret());
    let opened = open_token(first.token.expose_secret(), Session, &keys, now).unwrap();
    assert_eq!([opened.iat, opened.exp], [NOW, NOW + 3_600]);
    assert_eq!(opened.jti.len(), 22);
}

#[rstest]
#[case::access("llm_session_x", true, false)]
#[case::undecodable_access("llm_session_%%%", true, false)]
#[case::refresh("llm_srefresh_x", false, true)]
#[case::envelope("llm_env_x", false, false)]
#[case::upper_case("LLM_SESSION_x", false, false)]
#[case::empty("", false, false)]
fn routes_by_prefix(#[case] candidate: &str, #[case] access: bool, #[case] refresh: bool) {
    assert_eq!(candidate.is_session_token(), access);
    assert_eq!(candidate.is_session_refresh_token(), refresh);
}

#[rstest]
#[case::pkcs8(PRIVATE)]
#[case::pkcs1(PKCS1)]
fn derives_the_public_half(#[case] private: &str) {
    let keys = asymmetric(private, "current", Vec::new()).unwrap();
    assert_eq!(keys.public_key_pem().unwrap(), PUBLIC);
}

#[rstest]
#[case::short_secret(SessionKeys::new(SecretString::from("x".repeat(31))).map(drop), "Err(ShortSigningKey)")]
#[case::multibyte_secret(SessionKeys::new(SecretString::from("\u{e9}".repeat(32))).map(drop), "Ok(())")]
#[case::weak_rsa(asymmetric(WEAK, "k", Vec::new()).map(drop), "Err(PrivateKey")]
#[case::public_as_private(asymmetric(PUBLIC, "k", Vec::new()).map(drop), "Err(PrivateKey")]
#[case::not_pem(asymmetric("key", "k", Vec::new()).map(drop), "Err(Pem")]
#[case::empty_kid(asymmetric(PRIVATE, "", Vec::new()).map(drop), "Err(EmptyKid)")]
#[case::duplicate_kid(rotated("k", PREVIOUS_PUBLIC).and_then(|previous| asymmetric(PRIVATE, "k", previous)).map(drop), r#"Err(DuplicateKid { kids: "k" })"#)]
#[case::weak_public(rotated("old", WEAK_PUBLIC).map(drop), "Err(PublicKey")]
#[case::private_as_public(rotated("old", PRIVATE).map(drop), "Err(PublicKey")]
#[case::empty_public_kid(rotated("", PREVIOUS_PUBLIC).map(drop), "Err(EmptyKid)")]
fn validates_key_material(#[case] built: Result<(), SessionKeyError>, #[case] expected: &str) {
    assert!(format!("{built:?}").starts_with(expected), "{built:?}");
}
