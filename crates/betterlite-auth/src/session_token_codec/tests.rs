#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::sealing::seal;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, NonceSource, StoredCipher};
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::builders::{CliSession, SessionKind, SessionTokenCodec, SessionTokenError};
use super::record::SessionRecord;
use super::{
    DecodeError, LITELLM_SESSION_TOKEN_PREFIX, MAX_SESSION_JSON_DEPTH, MAX_SESSION_TOKEN_BYTES,
    decode_session_token,
};

const SALT: &str = "salt-one";
const UI_FIXED_TOKEN: &str = include_str!("../../fuzz/corpus/session_token_codec/ui-fixed-token");
const UI_MILLISECOND_TOKEN: &str =
    include_str!("../../fuzz/corpus/session_token_codec/ui-millisecond-token");
const CLI_DEFAULT_TOKEN: &str =
    include_str!("../../fuzz/corpus/session_token_codec/cli-default-token");
const CLI_EXPLICIT_TOKEN: &str =
    include_str!("../../fuzz/corpus/session_token_codec/cli-explicit-token");

struct FixedNonces;

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        nonce
            .iter_mut()
            .zip(0_u8..)
            .for_each(|(byte, index)| *byte = index);
        Ok(())
    }
}

type Codec = SessionTokenCodec<FixedNonces, TracingDiagnostics>;

fn cipher(key: Option<&str>) -> StoredCipher<FixedNonces, TracingDiagnostics> {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(SecretString::from),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: FixedNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn codec(key: Option<&str>, budget: Option<Decimal>, hours: i64) -> Codec {
    SessionTokenCodec {
        cipher: cipher(key),
        max_ui_session_budget: budget,
        cli_expiration_hours: hours,
    }
}

fn user(role: Option<&str>, models: &[&str], teams: &[&str]) -> LiteLLMUserTable {
    let role_text = role.map_or_else(|| "null".to_owned(), |text| format!("\"{text}\""));
    let models_text = serde_json::to_string(models).unwrap();
    let teams_text = serde_json::to_string(teams).unwrap();
    let row = format!(
        "{{\"user_id\":\"user-1\",\"user_role\":{role_text},\"models\":{models_text},\"teams\":{teams_text}}}"
    );
    serde_json::from_str(&row).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-12T17:50:00.123456+00:00")
        .unwrap()
        .with_timezone(&Utc)
}

fn payload(settings: &Codec, token: &SecretString) -> BTreeMap<String, JsonData> {
    decode_session_token(&settings.cipher, token.expose_secret().as_bytes()).unwrap()
}

fn field(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn at<'record>(payload: &'record BTreeMap<String, JsonData>, name: &str) -> &'record JsonData {
    payload.get(name).unwrap()
}

fn sealed(settings: &Codec, plaintext: &[u8], aad: &[u8]) -> String {
    let key = settings.cipher.settings.signing_key.as_ref().unwrap();
    let sealed = seal(
        Algorithm::Aes256Gcm,
        key,
        &settings.cipher.nonces,
        plaintext,
        aad,
    )
    .unwrap();
    format!(
        "{LITELLM_SESSION_TOKEN_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(sealed)
    )
}

#[test]
fn ui_fixed_fields() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let token = settings
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), now())
        .unwrap();
    assert_eq!(token.expose_secret(), UI_FIXED_TOKEN);
    let record = payload(&settings, &token);
    assert_eq!(at(&record, "token"), &field("\"ui-token\""));
    assert_eq!(at(&record, "key_name"), &field("\"ui-token\""));
    assert_eq!(at(&record, "key_alias"), &field("\"ui-token\""));
    assert_eq!(at(&record, "rpm_limit"), &field("100"));
    assert_eq!(at(&record, "team_id"), &field("\"litellm-dashboard\""));
    assert_eq!(at(&record, "models"), &field("[\"gpt-4o\"]"));
    assert_eq!(at(&record, "user_role"), &field("\"internal_user\""));
    assert_eq!(at(&record, "is_session_token"), &field("false"));
}

#[test]
fn ui_budget_override() {
    let overridden = codec(Some(SALT), Some("2.5".parse::<Decimal>().unwrap()), 24);
    let overridden_token = overridden
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), now())
        .unwrap();
    assert_eq!(
        at(&payload(&overridden, &overridden_token), "max_budget"),
        &field("2.5")
    );

    let disabled = codec(Some(SALT), None, 24);
    let disabled_token = disabled
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), now())
        .unwrap();
    assert!(!payload(&disabled, &disabled_token).contains_key("max_budget"));
}

#[rstest]
#[case::one_hour(1)]
#[case::ten_thousand_hours(10_000)]
fn ui_ignores_session_duration(#[case] cli_hours: i64) {
    let settings = codec(Some(SALT), Some(Decimal::ONE), cli_hours);
    let token = settings
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), now())
        .unwrap();
    assert_eq!(
        at(&payload(&settings, &token), "expires"),
        &field("\"2026-11-12T18:00:00.123+00:00\"")
    );
}

#[test]
fn expiry_milliseconds() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let micros = DateTime::parse_from_rfc3339("2026-11-12T17:50:00.999999+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let micro_token = settings
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), micros)
        .unwrap();
    assert_eq!(micro_token.expose_secret(), UI_MILLISECOND_TOKEN);
    assert_eq!(
        at(&payload(&settings, &micro_token), "expires"),
        &field("\"2026-11-12T18:00:00.999+00:00\"")
    );
    let whole = DateTime::parse_from_rfc3339("2026-11-12T17:50:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let whole_token = settings
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), whole)
        .unwrap();
    assert_eq!(
        at(&payload(&settings, &whole_token), "expires"),
        &field("\"2026-11-12T18:00:00.000+00:00\"")
    );
}

#[test]
fn missing_ui_role() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let error = settings
        .ui_session_token(&user(None, &[], &[]), now())
        .unwrap_err();
    assert!(matches!(
        error,
        SessionTokenError::MissingRole {
            session: SessionKind::ExperimentalUi
        }
    ));
}

#[test]
fn missing_cli_role() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let error = settings
        .cli_session_token(&user(None, &[], &[]), &CliSession::default(), now())
        .unwrap_err();
    assert!(matches!(
        error,
        SessionTokenError::MissingRole {
            session: SessionKind::Cli
        }
    ));
}

#[test]
fn invalid_role() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let invalid = user(Some("not-a-role"), &[], &[]);
    assert!(matches!(
        settings.ui_session_token(&invalid, now()).unwrap_err(),
        SessionTokenError::InvalidRole { .. }
    ));
    assert!(matches!(
        settings
            .cli_session_token(&invalid, &CliSession::default(), now())
            .unwrap_err(),
        SessionTokenError::InvalidRole { .. }
    ));
}

#[rstest]
#[case::five_hours(5, "2026-11-12T22:50:00.123+00:00")]
#[case::default_hours(24, "2026-11-13T17:50:00.123+00:00")]
#[case::zero_hours(0, "2026-11-12T17:50:00.123+00:00")]
#[case::negative_hours(-3, "2026-11-12T14:50:00.123+00:00")]
fn cli_session_expiry(#[case] hours: i64, #[case] expires: &str) {
    let settings = codec(Some(SALT), Some(Decimal::ONE), hours);
    let token = settings
        .cli_session_token(
            &user(Some("internal_user"), &[], &[]),
            &CliSession::default(),
            now(),
        )
        .unwrap();
    assert_eq!(
        at(&payload(&settings, &token), "expires"),
        &field(&format!("\"{expires}\""))
    );
}

#[test]
fn cli_first_team() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let token = settings
        .cli_session_token(
            &user(Some("internal_user"), &["gpt-4o"], &["team-a", "team-b"]),
            &CliSession::default(),
            now(),
        )
        .unwrap();
    assert_eq!(token.expose_secret(), CLI_DEFAULT_TOKEN);
    let record = payload(&settings, &token);
    assert_eq!(at(&record, "team_id"), &field("\"team-a\""));
    assert_eq!(at(&record, "models"), &field("[]"));
    assert_eq!(at(&record, "key_name"), &field("\"cli-session-user-1\""));
    assert_eq!(
        at(&record, "token"),
        &field("\"cli-session-AAECAwQFBgcICQoLDA0ODw\"")
    );
    assert_eq!(at(&record, "is_session_token"), &field("true"));
}

#[test]
fn cli_explicit_team() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let selection = CliSession {
        team_id: Some("team-explicit".to_owned()),
        team_alias: Some("Team Alias".to_owned()),
        team_models: Some(vec!["gpt-4o".to_owned(), "claude".to_owned()]),
        team_model_aliases: Some(BTreeMap::from([("gpt-4o".to_owned(), "alias".to_owned())])),
        max_budget: Some("5.5".parse::<Decimal>().unwrap()),
    };
    let explicit_token = settings
        .cli_session_token(
            &user(Some("internal_user"), &["gpt-4o", "claude"], &["team-a"]),
            &selection,
            now(),
        )
        .unwrap();
    assert_eq!(explicit_token.expose_secret(), CLI_EXPLICIT_TOKEN);
    let explicit_record = payload(&settings, &explicit_token);
    assert_eq!(at(&explicit_record, "team_id"), &field("\"team-explicit\""));
    assert_eq!(at(&explicit_record, "models"), &field("[]"));

    let empty = CliSession {
        team_id: Some(String::new()),
        team_alias: None,
        team_models: None,
        team_model_aliases: None,
        max_budget: None,
    };
    let empty_token = settings
        .cli_session_token(
            &user(Some("internal_user"), &["gpt-4o"], &["team-a"]),
            &empty,
            now(),
        )
        .unwrap();
    let empty_record = payload(&settings, &empty_token);
    assert_eq!(at(&empty_record, "team_id"), &field("\"\""));
    assert_eq!(at(&empty_record, "models"), &field("[]"));
}

#[test]
fn cli_without_team() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let token = settings
        .cli_session_token(
            &user(Some("proxy_admin"), &["gpt-4o"], &[]),
            &CliSession::default(),
            now(),
        )
        .unwrap();
    let record = payload(&settings, &token);
    assert!(!record.contains_key("team_id"));
    assert_eq!(at(&record, "models"), &field("[\"gpt-4o\"]"));
}

#[test]
fn cli_team_grants() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let selection = CliSession {
        team_id: None,
        team_alias: None,
        team_models: Some(vec!["gpt-4o".to_owned()]),
        team_model_aliases: Some(BTreeMap::from([(
            "gpt-4o".to_owned(),
            "gpt-alias".to_owned(),
        )])),
        max_budget: None,
    };
    let granted_token = settings
        .cli_session_token(&user(Some("internal_user"), &[], &[]), &selection, now())
        .unwrap();
    let granted_record = payload(&settings, &granted_token);
    assert_eq!(at(&granted_record, "team_models"), &field("[\"gpt-4o\"]"));
    assert_eq!(
        at(&granted_record, "team_model_aliases"),
        &field("{\"gpt-4o\":\"gpt-alias\"}")
    );

    let plain_token = settings
        .cli_session_token(
            &user(Some("internal_user"), &[], &[]),
            &CliSession::default(),
            now(),
        )
        .unwrap();
    let plain_record = payload(&settings, &plain_token);
    assert_eq!(at(&plain_record, "team_models"), &field("[]"));
    assert!(!plain_record.contains_key("team_model_aliases"));
}

#[test]
fn cli_alias_and_budget() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let selection = CliSession {
        team_id: None,
        team_alias: Some("Team Alias".to_owned()),
        team_models: None,
        team_model_aliases: None,
        max_budget: Some("5.5".parse::<Decimal>().unwrap()),
    };
    let granted_token = settings
        .cli_session_token(&user(Some("internal_user"), &[], &[]), &selection, now())
        .unwrap();
    let granted_record = payload(&settings, &granted_token);
    assert_eq!(at(&granted_record, "team_alias"), &field("\"Team Alias\""));
    assert_eq!(at(&granted_record, "max_budget"), &field("5.5"));

    let plain_token = settings
        .cli_session_token(
            &user(Some("internal_user"), &[], &[]),
            &CliSession::default(),
            now(),
        )
        .unwrap();
    let plain_record = payload(&settings, &plain_token);
    assert!(!plain_record.contains_key("team_alias"));
    assert!(!plain_record.contains_key("max_budget"));
}

#[test]
fn null_fields_omitted() {
    let settings = codec(Some(SALT), None, 24);
    let token = settings
        .ui_session_token(&user(Some("internal_user"), &[], &[]), now())
        .unwrap();
    let record = payload(&settings, &token);
    assert!(!record.contains_key("max_parallel_requests"));
    assert!(!record.contains_key("max_budget"));
    assert!(!record.contains_key("team_alias"));
    assert!(!record.contains_key("team_model_aliases"));
}

#[test]
fn both_sessions_clear_parallel_limit() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let ui_token = settings
        .ui_session_token(&user(Some("internal_user"), &[], &[]), now())
        .unwrap();
    assert!(!payload(&settings, &ui_token).contains_key("max_parallel_requests"));
    let cli_token = settings
        .cli_session_token(
            &user(Some("internal_user"), &[], &[]),
            &CliSession::default(),
            now(),
        )
        .unwrap();
    assert!(!payload(&settings, &cli_token).contains_key("max_parallel_requests"));
}

#[test]
fn ui_token_roundtrip() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let token = settings
        .ui_session_token(&user(Some("internal_user"), &["gpt-4o"], &[]), now())
        .unwrap();
    let auth = settings
        .session_from_token(token.expose_secret())
        .unwrap()
        .unwrap();
    let record = serde_json::to_value(SessionRecord::from_auth(&auth)).unwrap();
    assert_eq!(record["token"].as_str(), Some("ui-token"));
    assert_eq!(record["key_alias"].as_str(), Some("ui-token"));
    assert_eq!(record["user_role"].as_str(), Some("internal_user"));
    assert_eq!(record["team_id"].as_str(), Some("litellm-dashboard"));
    assert_eq!(record["rpm_limit"].as_i64(), Some(100));
    assert_eq!(record["is_session_token"].as_bool(), Some(false));
    assert_eq!(
        record["expires"].as_str(),
        Some("2026-11-12T18:00:00.123+00:00")
    );
    assert!(record["max_parallel_requests"].is_null());
}

#[test]
fn bearer_prefix_aad() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let token = settings
        .ui_session_token(&user(Some("internal_user"), &[], &[]), now())
        .unwrap();
    assert!(
        settings
            .session_from_token(token.expose_secret())
            .unwrap()
            .is_some()
    );
    let foreign = sealed(&settings, b"{\"token\":\"ui-token\"}", b"other_");
    assert!(settings.session_from_token(&foreign).unwrap().is_none());
    assert_eq!(
        decode_session_token(&settings.cipher, foreign.as_bytes())
            .unwrap_err()
            .to_string(),
        "the session token is not a bearer token"
    );
}

#[test]
fn bearer_missing_secret() {
    let settings = codec(None, Some(Decimal::ONE), 24);
    assert!(matches!(
        settings
            .ui_session_token(&user(Some("internal_user"), &[], &[]), now())
            .unwrap_err(),
        SessionTokenError::MissingSecret
    ));
    assert!(matches!(
        settings
            .cli_session_token(
                &user(Some("internal_user"), &[], &[]),
                &CliSession::default(),
                now()
            )
            .unwrap_err(),
        SessionTokenError::MissingSecret
    ));
    assert!(
        settings
            .session_from_token(UI_FIXED_TOKEN)
            .unwrap()
            .is_none()
    );
}

#[test]
fn bearer_empty_secret() {
    let settings = codec(Some(""), Some(Decimal::ONE), 24);
    let token = settings
        .ui_session_token(&user(Some("internal_user"), &[], &[]), now())
        .unwrap();
    assert_ne!(token.expose_secret(), UI_FIXED_TOKEN);
    let record = payload(&settings, &token);
    assert_eq!(at(&record, "token"), &field("\"ui-token\""));
    assert!(
        settings
            .session_from_token(token.expose_secret())
            .unwrap()
            .is_some()
    );
    let under_salt = codec(Some(SALT), Some(Decimal::ONE), 24);
    assert!(
        under_salt
            .session_from_token(token.expose_secret())
            .unwrap()
            .is_none()
    );
}

#[test]
fn decrypt_wrong_prefix() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let bare = UI_FIXED_TOKEN
        .strip_prefix(LITELLM_SESSION_TOKEN_PREFIX)
        .unwrap();
    assert!(settings.session_from_token(bare).unwrap().is_none());
    let other = format!("other_{bare}");
    assert!(settings.session_from_token(&other).unwrap().is_none());
}

#[test]
fn decrypt_bad_base64() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let token = format!("{LITELLM_SESSION_TOKEN_PREFIX}%%%%");
    assert!(settings.session_from_token(&token).unwrap().is_none());
    assert_eq!(
        decode_session_token(&settings.cipher, token.as_bytes())
            .unwrap_err()
            .to_string(),
        "the session token is not a bearer token"
    );
}

#[rstest]
#[case::three_bytes(&format!("{LITELLM_SESSION_TOKEN_PREFIX}AAAA"))]
#[case::prefix_only(LITELLM_SESSION_TOKEN_PREFIX)]
fn decrypt_short_ciphertext(#[case] token: &str) {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    assert!(settings.session_from_token(token).unwrap().is_none());
}

#[test]
fn decrypt_wrong_key() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let wrong = seal(
        Algorithm::Aes256Gcm,
        &SecretString::from("wrong-key"),
        &settings.cipher.nonces,
        b"{\"token\":\"ui-token\"}",
        LITELLM_SESSION_TOKEN_PREFIX.as_bytes(),
    )
    .unwrap();
    let token = format!(
        "{LITELLM_SESSION_TOKEN_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(wrong)
    );
    assert!(settings.session_from_token(&token).unwrap().is_none());
}

#[test]
fn decrypt_bad_utf8() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let bad_utf8 = sealed(
        &settings,
        b"\xff\xfe",
        LITELLM_SESSION_TOKEN_PREFIX.as_bytes(),
    );
    assert!(settings.session_from_token(&bad_utf8).unwrap().is_none());
    let invalid_token = [LITELLM_SESSION_TOKEN_PREFIX.as_bytes(), b"\xff"].concat();
    assert!(matches!(
        decode_session_token(&settings.cipher, &invalid_token).unwrap_err(),
        DecodeError::Utf8 { .. }
    ));
}

#[test]
fn malformed_session_json() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let malformed = sealed(
        &settings,
        b"not json",
        LITELLM_SESSION_TOKEN_PREFIX.as_bytes(),
    );
    assert!(matches!(
        settings.session_from_token(&malformed).unwrap_err(),
        SessionTokenError::Payload {
            source: DecodeError::Json { .. }
        }
    ));
}

#[test]
fn invalid_session_fields() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let prefix = LITELLM_SESSION_TOKEN_PREFIX.as_bytes();
    let bad_type = sealed(&settings, b"{\"token\":123}", prefix);
    assert!(matches!(
        settings.session_from_token(&bad_type).unwrap_err(),
        SessionTokenError::Record { .. }
    ));
    let invalid_role = sealed(&settings, b"{\"user_role\":\"not-a-role\"}", prefix);
    assert!(matches!(
        settings.session_from_token(&invalid_role).unwrap_err(),
        SessionTokenError::Record { .. }
    ));
    let wrong_aad = sealed(&settings, b"{\"token\":\"ui-token\"}", b"other_");
    assert!(settings.session_from_token(&wrong_aad).unwrap().is_none());
}

#[test]
fn session_input_limits() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    byte_limit(&settings);
    json_depth_limit(&settings);
}

fn byte_limit(settings: &Codec) {
    let boundary = vec![b'a'; MAX_SESSION_TOKEN_BYTES];
    assert!(!matches!(
        decode_session_token(&settings.cipher, &boundary),
        Err(DecodeError::TooLarge { .. })
    ));
    let oversized = vec![b'a'; MAX_SESSION_TOKEN_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_session_token(&settings.cipher, &oversized).unwrap_err(),
        DecodeError::TooLarge { .. }
    ));
    assert!(matches!(
        settings
            .session_from_token(&String::from_utf8(oversized).unwrap())
            .unwrap_err(),
        SessionTokenError::Payload {
            source: DecodeError::TooLarge { .. }
        }
    ));
}

fn json_depth_limit(settings: &Codec) {
    let allowed = format!(
        "{{\"a\":{}{}}}",
        "[".repeat(MAX_SESSION_JSON_DEPTH.saturating_sub(1)),
        "]".repeat(MAX_SESSION_JSON_DEPTH.saturating_sub(1))
    );
    let allowed_token = sealed(
        settings,
        allowed.as_bytes(),
        LITELLM_SESSION_TOKEN_PREFIX.as_bytes(),
    );
    drop(decode_session_token(&settings.cipher, allowed_token.as_bytes()).unwrap());
    let too_deep = format!(
        "{{\"a\":{}{}}}",
        "[".repeat(MAX_SESSION_JSON_DEPTH),
        "]".repeat(MAX_SESSION_JSON_DEPTH)
    );
    let deep_token = sealed(
        settings,
        too_deep.as_bytes(),
        LITELLM_SESSION_TOKEN_PREFIX.as_bytes(),
    );
    assert!(matches!(
        decode_session_token(&settings.cipher, deep_token.as_bytes()).unwrap_err(),
        DecodeError::TooDeep { .. }
    ));
}

#[derive(Clone, Default)]
struct Events(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for Events {
    fn on_event(&self, _event: &Event<'_>, _: Context<'_, S>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn session_error_redaction() {
    let settings = codec(Some(SALT), Some(Decimal::ONE), 24);
    let plaintext = b"{\"key_alias\":\"SECRET-MARKER\",\"user_role\":\"not-a-role\"}";
    let token = sealed(
        &settings,
        plaintext,
        LITELLM_SESSION_TOKEN_PREFIX.as_bytes(),
    );
    let events = Events::default();
    let subscriber = tracing_subscriber::registry().with(events.clone());
    let error = with_default(subscriber, || {
        settings.session_from_token(&token).unwrap_err()
    });
    let rendered = format!("{error} {error:?}");
    let ciphertext = token.strip_prefix(LITELLM_SESSION_TOKEN_PREFIX).unwrap();
    assert!(!rendered.contains(ciphertext));
    assert!(!rendered.contains("SECRET-MARKER"));
    // the decode failure logs nothing at all, so no tracing line can carry the token or the record
    assert_eq!(events.0.load(Ordering::Relaxed), 0);
}

#[rstest]
fn replays_the_session_token_codec_corpus(
    #[files("fuzz/corpus/session_token_codec/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_session_token(&cipher(Some(SALT)), &bytes));
}
