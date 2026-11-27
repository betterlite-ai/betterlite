#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::sso_types::identity::{LoginMethod, ReturnedUiTokenObject};
use betterlite_config::sections::key_access::{
    KeyGeneration, PersonalKeyGeneration, TeamKeyGeneration,
};
use betterlite_config::sections::ui_session_token::Settings;
use betterlite_db::managed_resource_rows::wire::RowFlag;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use jsonwebtoken::errors::{Error as JwtError, ErrorKind};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;

use super::{
    UiSessionTokenError, disabled_non_admin_personal_key_creation, encode_ui_session_jwt,
    ui_session_exp_timestamp,
};

const MASTER: &str = "sk-master-secret";
const VIRTUAL_KEY: &str = "sk-virtual-secret";
const NOW: &str = "2026-01-01T12:00:00.999999Z";

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DecodedClaims {
    user_id: String,
    key: String,
    user_email: Option<String>,
    user_role: String,
    login_method: String,
    premium_user: RowFlag,
    auth_header_name: String,
    disabled_non_admin_personal_key_creation: RowFlag,
    server_root_path: String,
    password_reset_required: RowFlag,
    exp: i64,
}

fn now() -> DateTime<Utc> {
    NOW.parse().unwrap()
}

fn settings(duration: &str) -> Settings {
    Settings {
        duration: duration.to_owned(),
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn sample_token(key: &str, email: Option<&str>, method: LoginMethod) -> ReturnedUiTokenObject {
    ReturnedUiTokenObject {
        user_id: "user-1".to_owned(),
        key: secret(key),
        user_email: email.map(str::to_owned),
        user_role: "internal_user".to_owned(),
        login_method: method,
        premium_user: RowFlag(true),
        auth_header_name: "Authorization".to_owned(),
        personal_key_creation_disabled: RowFlag(false),
        server_root_path: "/litellm".to_owned(),
        password_reset_required: RowFlag(false),
    }
}

fn decode_claims(token: &str, master_key: &str) -> DecodedClaims {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    decode::<DecodedClaims>(
        token,
        &DecodingKey::from_secret(master_key.as_bytes()),
        &validation,
    )
    .unwrap()
    .claims
}

fn segment(token: &str, index: usize) -> String {
    let encoded = token.split('.').nth(index).unwrap();
    let bytes = URL_SAFE_NO_PAD.decode(encoded).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn signs_every_field_of_an_sso_token() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, Some("user@example.com"), LoginMethod::Sso);
    let encoded =
        encode_ui_session_jwt(&token, &secret(MASTER), &settings("24h"), now(), &database).unwrap();
    let claims = decode_claims(&encoded, MASTER);
    assert_eq!(
        claims,
        DecodedClaims {
            user_id: "user-1".to_owned(),
            key: VIRTUAL_KEY.to_owned(),
            user_email: Some("user@example.com".to_owned()),
            user_role: "internal_user".to_owned(),
            login_method: "sso".to_owned(),
            premium_user: RowFlag(true),
            auth_header_name: "Authorization".to_owned(),
            disabled_non_admin_personal_key_creation: RowFlag(false),
            server_root_path: "/litellm".to_owned(),
            password_reset_required: RowFlag(false),
            exp: 1_767_355_200,
        }
    );
}

#[test]
fn signs_every_field_of_a_username_password_token() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(
        VIRTUAL_KEY,
        Some("user@example.com"),
        LoginMethod::UsernamePassword,
    );
    let encoded =
        encode_ui_session_jwt(&token, &secret(MASTER), &settings("24h"), now(), &database).unwrap();
    let claims = decode_claims(&encoded, MASTER);
    assert_eq!(claims.login_method, "username_password");
    assert_eq!(claims.key, VIRTUAL_KEY);
    assert_eq!(claims.exp, 1_767_355_200);
}

#[test]
fn writes_the_claims_in_the_token_object_order_with_exp_last() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, Some("user@example.com"), LoginMethod::Sso);
    let encoded =
        encode_ui_session_jwt(&token, &secret(MASTER), &settings("24h"), now(), &database).unwrap();
    assert_eq!(
        segment(&encoded, 1),
        concat!(
            r#"{"user_id":"user-1","key":"sk-virtual-secret","user_email":"user@example.com","#,
            r#""user_role":"internal_user","login_method":"sso","premium_user":true,"#,
            r#""auth_header_name":"Authorization","#,
            r#""disabled_non_admin_personal_key_creation":false,"server_root_path":"/litellm","#,
            r#""password_reset_required":false,"exp":1767355200}"#,
        )
    );
}

#[test]
fn writes_an_hs256_header() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let encoded =
        encode_ui_session_jwt(&token, &secret(MASTER), &settings("24h"), now(), &database).unwrap();
    let header = decode_header(&encoded).unwrap();
    assert_eq!(header.alg, Algorithm::HS256);
    assert_eq!(header.typ.as_deref(), Some("JWT"));
}

#[test]
fn keeps_a_null_email_as_null() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let encoded =
        encode_ui_session_jwt(&token, &secret(MASTER), &settings("24h"), now(), &database).unwrap();
    let claims = decode_claims(&encoded, MASTER);
    assert!(claims.user_email.is_none(), "{claims:?}");
}

#[test]
fn keeps_a_non_ascii_email() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(
        VIRTUAL_KEY,
        Some("jos\u{e9}@ex\u{e4}mple.com"),
        LoginMethod::Sso,
    );
    let encoded =
        encode_ui_session_jwt(&token, &secret(MASTER), &settings("24h"), now(), &database).unwrap();
    let claims = decode_claims(&encoded, MASTER);
    assert_eq!(
        claims.user_email.as_deref(),
        Some("jos\u{e9}@ex\u{e4}mple.com")
    );
}

// the expected epochs are the login instant with its fraction cut, plus each duration
#[rstest]
#[case::seconds("30s", 1_767_268_830)]
#[case::hours("24h", 1_767_355_200)]
#[case::daily_alias("daily", 1_767_355_200)]
#[case::weeks("2w", 1_768_478_400)]
fn cuts_the_fraction_and_adds_the_duration(#[case] duration: &str, #[case] expected: i64) {
    let database = TimeZoneDatabase::from_env();
    let exp = ui_session_exp_timestamp(&settings(duration), now(), &database).unwrap();
    assert_eq!(exp, expected);
}

#[rstest]
#[case::empty("")]
#[case::no_unit("24")]
#[case::text("abc")]
fn rejects_an_invalid_duration_at_encode_time(#[case] duration: &str) {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let error = encode_ui_session_jwt(
        &token,
        &secret(MASTER),
        &settings(duration),
        now(),
        &database,
    )
    .unwrap_err();
    assert!(matches!(error, UiSessionTokenError::Duration(_)), "{error}");
}

#[test]
fn rejects_a_duration_that_does_not_fit_a_timestamp() {
    let database = TimeZoneDatabase::from_env();
    let error =
        ui_session_exp_timestamp(&settings("9300000000000000s"), now(), &database).unwrap_err();
    assert!(matches!(error, UiSessionTokenError::Overflow), "{error}");
}

#[test]
fn refuses_an_empty_master_key() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let error =
        encode_ui_session_jwt(&token, &secret(""), &settings("24h"), now(), &database).unwrap_err();
    assert!(
        matches!(error, UiSessionTokenError::EmptyMasterKey),
        "{error}"
    );
}

// PyJWT 2.15.1 refuses a PEM-shaped HMAC key; betterlite never sniffs the key and signs with it
#[test]
fn signs_with_a_pem_shaped_master_key() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let pem = "-----BEGIN PUBLIC KEY-----\nMFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAK0=\n-----END PUBLIC KEY-----";
    let encoded =
        encode_ui_session_jwt(&token, &secret(pem), &settings("24h"), now(), &database).unwrap();
    assert_eq!(decode_claims(&encoded, pem).user_id, "user-1");
}

#[test]
fn checks_the_duration_before_the_master_key() {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let error =
        encode_ui_session_jwt(&token, &secret(""), &settings("abc"), now(), &database).unwrap_err();
    assert!(matches!(error, UiSessionTokenError::Duration(_)), "{error}");
}

#[derive(Clone, Copy, Debug)]
enum FailingCall {
    Duration,
    EmptyMasterKey,
    Overflow,
    Encode,
}

#[rstest]
#[case::invalid_duration(FailingCall::Duration)]
#[case::empty_master_key(FailingCall::EmptyMasterKey)]
#[case::overflow(FailingCall::Overflow)]
#[case::encode(FailingCall::Encode)]
fn error_text_never_carries_a_key(#[case] call: FailingCall) {
    let database = TimeZoneDatabase::from_env();
    let token = sample_token(VIRTUAL_KEY, None, LoginMethod::Sso);
    let error = match call {
        FailingCall::Duration => {
            encode_ui_session_jwt(&token, &secret(MASTER), &settings("abc"), now(), &database)
                .unwrap_err()
        }
        FailingCall::EmptyMasterKey => {
            encode_ui_session_jwt(&token, &secret(""), &settings("24h"), now(), &database)
                .unwrap_err()
        }
        FailingCall::Overflow => {
            ui_session_exp_timestamp(&settings("9300000000000000s"), now(), &database).unwrap_err()
        }
        FailingCall::Encode => UiSessionTokenError::Encode(JwtError::from(ErrorKind::InvalidToken)),
    };
    let text = format!("{error} {error:?}");
    assert!(!text.contains(MASTER), "{text}");
    assert!(!text.contains(VIRTUAL_KEY), "{text}");
}

fn personal(roles: Option<Vec<String>>) -> KeyGeneration {
    KeyGeneration {
        configured: true,
        team: TeamKeyGeneration::default(),
        personal: Some(PersonalKeyGeneration {
            allowed_user_roles: roles,
            required_params: None,
        }),
    }
}

fn configured_without_personal() -> KeyGeneration {
    KeyGeneration {
        configured: true,
        team: TeamKeyGeneration::default(),
        personal: None,
    }
}

#[rstest]
#[case::unconfigured(KeyGeneration::default(), false)]
#[case::without_personal(configured_without_personal(), false)]
#[case::without_roles(personal(None), false)]
fn reports_no_disabled_creation_without_roles(
    #[case] key_generation: KeyGeneration,
    #[case] expected: bool,
) {
    assert_eq!(
        disabled_non_admin_personal_key_creation(&key_generation),
        expected
    );
}

#[rstest]
#[case::admin_only(Some(vec!["proxy_admin".to_owned()]), true)]
#[case::admin_after_other_roles(
    Some(vec!["internal_user".to_owned(), "proxy_admin".to_owned()]),
    true
)]
#[case::admin_in_other_case(Some(vec!["PROXY_ADMIN".to_owned()]), false)]
#[case::empty_roles(Some(Vec::new()), false)]
fn reads_the_proxy_admin_role(#[case] roles: Option<Vec<String>>, #[case] expected: bool) {
    assert_eq!(
        disabled_non_admin_personal_key_creation(&personal(roles)),
        expected
    );
}
