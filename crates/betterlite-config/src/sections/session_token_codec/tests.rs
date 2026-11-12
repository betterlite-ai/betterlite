#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::session_token_codec::builders::{CliSession, SessionTokenCodec};
use betterlite_auth::session_token_codec::decode_session_token;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::ConfigLoader;
use crate::types::Config;

const SALT: &str = "salt-one";

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

fn load(source: &str, variables: &[(&str, &str)]) -> Config {
    let mut lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    // without a key no test below reaches the builder; an explicit variable still overrides this
    lookup
        .entry("LITELLM_SALT_KEY".to_owned())
        .or_insert_with(|| SALT.to_owned());
    ConfigLoader { lookup }.load(source).unwrap()
}

fn codec(config: &Config) -> SessionTokenCodec<FixedNonces, TracingDiagnostics> {
    SessionTokenCodec {
        cipher: StoredCipher {
            settings: config.general_settings.cipher.clone(),
            nonces: FixedNonces,
            diagnostics: TracingDiagnostics,
        },
        max_ui_session_budget: config.sections.session_token_codec.max_ui_session_budget,
        cli_expiration_hours: config.sections.session_token_codec.cli_expiration_hours,
    }
}

fn user() -> LiteLLMUserTable {
    let row = "{\"user_id\":\"user-1\",\"user_role\":\"internal_user\",\"models\":[\"gpt-4o\"]}";
    serde_json::from_str(row).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-12T17:50:00.123456+00:00")
        .unwrap()
        .with_timezone(&Utc)
}

fn record(config: &Config, token: &SecretString) -> BTreeMap<String, JsonData> {
    decode_session_token(&codec(config).cipher, token.expose_secret().as_bytes()).unwrap()
}

fn field(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn at<'record>(record: &'record BTreeMap<String, JsonData>, name: &str) -> &'record JsonData {
    record.get(name).unwrap()
}

#[test]
fn loads_settings() {
    let config = load(
        "general_settings: {master_key: other-key}\nlitellm_settings: {max_ui_session_budget: 2.5}",
        &[
            ("LITELLM_SALT_KEY", "salt-one"),
            ("CLI_JWT_EXPIRATION_HOURS", "5"),
            ("LITELLM_UI_SESSION_DURATION", "30m"),
        ],
    );
    assert_eq!(
        config.sections.session_token_codec.max_ui_session_budget,
        Some("2.5".parse::<Decimal>().unwrap())
    );
    assert_eq!(config.sections.session_token_codec.cli_expiration_hours, 5);
    let settings = codec(&config);
    let ui = settings.ui_session_token(&user(), now()).unwrap();
    let ui_record = record(&config, &ui);
    assert_eq!(at(&ui_record, "max_budget"), &field("2.5"));
    assert_eq!(
        at(&ui_record, "expires"),
        &field("\"2026-11-12T18:00:00.123+00:00\"")
    );
    let cli = settings
        .cli_session_token(&user(), &CliSession::default(), now())
        .unwrap();
    assert_eq!(
        at(&record(&config, &cli), "expires"),
        &field("\"2026-11-12T22:50:00.123+00:00\"")
    );
}

#[test]
fn legacy_hours_win() {
    let config = load(
        "litellm_settings: {}",
        &[
            ("CLI_JWT_EXPIRATION_HOURS", "5"),
            ("LITELLM_CLI_JWT_EXPIRATION_HOURS", "7"),
        ],
    );
    assert_eq!(config.sections.session_token_codec.cli_expiration_hours, 5);
    let token = codec(&config)
        .cli_session_token(&user(), &CliSession::default(), now())
        .unwrap();
    assert_eq!(
        at(&record(&config, &token), "expires"),
        &field("\"2026-11-12T22:50:00.123+00:00\"")
    );
}

#[rstest]
#[case::legacy_empty(
    &[("CLI_JWT_EXPIRATION_HOURS", ""), ("LITELLM_CLI_JWT_EXPIRATION_HOURS", "7")],
    7
)]
#[case::both_empty(
    &[("CLI_JWT_EXPIRATION_HOURS", ""), ("LITELLM_CLI_JWT_EXPIRATION_HOURS", "")],
    24
)]
#[case::both_absent(&[], 24)]
fn empty_hours_fall_through(#[case] variables: &[(&str, &str)], #[case] expected: i64) {
    let config = load("litellm_settings: {}", variables);
    assert_eq!(
        config.sections.session_token_codec.cli_expiration_hours,
        expected
    );
    let token = codec(&config)
        .cli_session_token(&user(), &CliSession::default(), now())
        .unwrap();
    let expected_expiry = match expected {
        7 => field("\"2026-11-13T00:50:00.123+00:00\""),
        _ => field("\"2026-11-13T17:50:00.123+00:00\""),
    };
    assert_eq!(at(&record(&config, &token), "expires"), &expected_expiry);
}

#[rstest]
#[case::legacy_invalid(&[("CLI_JWT_EXPIRATION_HOURS", "abc")], "CLI_JWT_EXPIRATION_HOURS")]
#[case::fallback_invalid(
    &[("CLI_JWT_EXPIRATION_HOURS", ""), ("LITELLM_CLI_JWT_EXPIRATION_HOURS", "abc")],
    "LITELLM_CLI_JWT_EXPIRATION_HOURS"
)]
fn invalid_hours_stop_startup(#[case] variables: &[(&str, &str)], #[case] variable: &str) {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let error = ConfigLoader { lookup }
        .load("litellm_settings: {}")
        .unwrap_err();
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains(variable));
}

#[rstest]
#[case::zero("0", "\"2026-11-12T17:50:00.123+00:00\"")]
#[case::negative("-3", "\"2026-11-12T14:50:00.123+00:00\"")]
#[case::plus("+5", "\"2026-11-12T22:50:00.123+00:00\"")]
fn signed_hours_are_accepted(#[case] hours: &str, #[case] expires: &str) {
    let config = load(
        "litellm_settings: {}",
        &[("CLI_JWT_EXPIRATION_HOURS", hours)],
    );
    assert_eq!(
        config.sections.session_token_codec.cli_expiration_hours,
        hours.parse::<i64>().unwrap()
    );
    let token = codec(&config)
        .cli_session_token(&user(), &CliSession::default(), now())
        .unwrap();
    assert_eq!(at(&record(&config, &token), "expires"), &field(expires));
}

#[rstest]
#[case::absent("litellm_settings: {}", true)]
#[case::explicit_null("litellm_settings: {max_ui_session_budget: null}", false)]
#[case::explicit_zero("litellm_settings: {max_ui_session_budget: 0}", true)]
fn ui_budget_nullable(#[case] source: &str, #[case] budget_present: bool) {
    let config = load(source, &[]);
    let settings = codec(&config);
    assert_eq!(
        config
            .sections
            .session_token_codec
            .max_ui_session_budget
            .is_some(),
        budget_present
    );
    let token = settings.ui_session_token(&user(), now()).unwrap();
    let ui_record = record(&config, &token);
    assert_eq!(ui_record.contains_key("max_budget"), budget_present);
    if let Some(budget) = config.sections.session_token_codec.max_ui_session_budget {
        let expected = budget.to_string().parse::<f64>().unwrap();
        assert_eq!(
            serde_json::to_string(at(&ui_record, "max_budget")).unwrap(),
            serde_json::to_string(&expected).unwrap()
        );
    }
}

#[test]
fn salt_before_master() {
    let source = "general_settings: {master_key: other-key}";
    let salted = load(source, &[("LITELLM_SALT_KEY", SALT)]);
    // an empty lookup has no salt at all, so only the master key can encrypt here
    let master_only = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap();
    let token = codec(&salted).ui_session_token(&user(), now()).unwrap();
    assert!(
        codec(&salted)
            .session_from_token(token.expose_secret())
            .unwrap()
            .is_some()
    );
    assert!(
        codec(&master_only)
            .session_from_token(token.expose_secret())
            .unwrap()
            .is_none()
    );

    let empty = load(source, &[("LITELLM_SALT_KEY", "")]);
    let empty_token = codec(&empty).ui_session_token(&user(), now()).unwrap();
    assert!(
        codec(&empty)
            .session_from_token(empty_token.expose_secret())
            .unwrap()
            .is_some()
    );
    assert!(
        codec(&salted)
            .session_from_token(empty_token.expose_secret())
            .unwrap()
            .is_none()
    );
}
