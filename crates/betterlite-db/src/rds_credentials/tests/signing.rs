#![cfg(test)]

use std::time::Duration;

use aws_config::timeout::TimeoutConfig;
use percent_encoding::percent_decode_str;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::fixture::{
    BotocoreFixture, Clock, Preset, RecordingSecrets, Reply, SECRET_KEY, SIGNER_CREDENTIALS,
    TokenVector, aws_sources, botocore_fixture, endpoint, environment, preset_inputs, secret,
};
use crate::rds_credentials::error::RdsCredentialsError;
use crate::rds_credentials::{
    RdsCredentialInputs, RdsSigner, RdsTimeout, generate_iam_auth_token, rds_timeout_config,
};

fn vector_inputs(vector: &TokenVector) -> RdsCredentialInputs {
    RdsCredentialInputs {
        access_key_id: Some(secret(&vector.access_key_id)),
        secret_access_key: Some(secret(&vector.secret_access_key)),
        region_name: Some(vector.region.clone()),
        session_name: None,
        profile_name: None,
        role_name: None,
        web_identity_token: None,
        timeout: RdsTimeout::SdkDefaults,
    }
}

fn vector_reply(vector: &TokenVector) -> Reply {
    match vector.source.as_str() {
        "assume_role" => Reply::AssumeRole,
        _ => Reply::WebIdentity,
    }
}

fn vector_source_inputs(vector: &TokenVector) -> RdsCredentialInputs {
    match vector.source.as_str() {
        "web_identity" => preset_inputs(Preset::WebIdentity),
        "assume_role" => preset_inputs(Preset::AssumeRole),
        _ => vector_inputs(vector),
    }
}

async fn assert_vector(pinned: &BotocoreFixture, name: &str) {
    let vector = pinned
        .tokens
        .iter()
        .find(|vector| vector.name == name)
        .expect("a pinned botocore vector");
    let secrets = RecordingSecrets::new(&[]);
    let environment = environment(&SIGNER_CREDENTIALS);
    let clock = Clock::at(pinned.fixed_time_unix_seconds);
    let (sources, _recorder) = aws_sources(
        &secrets,
        &environment,
        &[("/token", "oidc-token-value")],
        &clock,
        vector_reply(vector),
    );
    let signer = RdsSigner::init(vector_source_inputs(vector), sources)
        .await
        .expect("the pinned credentials sign");
    let token = signer
        .db_auth_token(&endpoint(&vector.host, &vector.port, &vector.user))
        .expect("the token presigns");
    assert_eq!(
        token.expose_secret(),
        vector.token,
        "vector {}",
        vector.name
    );
    match &vector.session_token {
        Some(session_token) => assert!(
            token
                .expose_secret()
                .contains(&format!("X-Amz-Security-Token={session_token}"))
        ),
        None => assert!(!token.expose_secret().contains("X-Amz-Security-Token")),
    }
}

pub(super) async fn pinned_vectors() {
    let pinned = botocore_fixture();
    assert_vector(&pinned, "direct_credentials").await;
    assert_vector(&pinned, "no_session_token_special_user").await;
    assert_vector(&pinned, "empty_access_key").await;
    assert_vector(&pinned, "default_https_port").await;
    assert_vector(&pinned, "mixed_case_host").await;
    assert_vector(&pinned, "temporary_web_identity").await;
    assert_vector(&pinned, "temporary_assume_role").await;
}

#[tokio::test]
async fn signer_debug_hides_the_credentials() {
    let secrets = RecordingSecrets::new(&[]);
    let environment = environment(&[]);
    let clock = Clock::at(1_794_225_600);
    let (sources, _recorder) = aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let signer = RdsSigner::init(preset_inputs(Preset::Direct), sources)
        .await
        .unwrap();
    assert_eq!(format!("{signer:?}"), "RdsSigner");
    assert!(!format!("{signer:?}").contains(SECRET_KEY));
}

#[test]
fn fixture_provenance_pins_botocore() {
    let pinned = botocore_fixture();
    assert_eq!(pinned.provenance.python, "3.13");
    assert_eq!(pinned.provenance.boto3, "1.43.118");
    assert_eq!(pinned.provenance.botocore, "1.43.118");
    assert_eq!(pinned.provenance.fixed_time, "2026-11-09T12:00:00Z");
    assert_eq!(pinned.fixed_time_unix_seconds, 1_794_225_600);
    assert_eq!(pinned.sts.region, "us-east-1");
}

#[tokio::test]
async fn signer_receives_original_endpoint() {
    let secrets = RecordingSecrets::new(&[]);
    let environment = environment(&[]);
    let clock = Clock::at(1_794_225_600);
    let (sources, _recorder) = aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let signer = RdsSigner::init(preset_inputs(Preset::Direct), sources)
        .await
        .unwrap();
    let special = signer
        .db_auth_token(&endpoint(
            "db.example.com",
            "5432",
            "user@example.com/weird +name",
        ))
        .unwrap();
    assert!(special.expose_secret().starts_with(
        "db.example.com:5432/?Action=connect&DBUser=user%40example.com%2Fweird%20%2Bname&X-Amz-Algorithm="
    ));
    let encoded_user = signer
        .db_auth_token(&endpoint("db.example.com", "5432", "user%40example.com"))
        .unwrap();
    assert!(
        encoded_user
            .expose_secret()
            .contains("DBUser=user%2540example.com&")
    );
    let no_port = signer
        .db_auth_token(&endpoint("db.example.com", "", "admin"))
        .unwrap();
    assert!(
        no_port
            .expose_secret()
            .starts_with("db.example.com:/?Action=connect&DBUser=admin&")
    );
}

#[tokio::test]
async fn whole_token_percent_encoding() {
    let secrets = RecordingSecrets::new(&[]);
    let environment = environment(&[]);
    let clock = Clock::at(1_794_225_600);
    let (sources, _recorder) = aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let signer = RdsSigner::init(preset_inputs(Preset::Direct), sources)
        .await
        .unwrap();
    let endpoint = endpoint("db.example.com", "5432", "user@example.com/weird +name");
    let raw = signer.db_auth_token(&endpoint).unwrap();
    let (mint_sources, _mint_recorder) =
        aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let password = generate_iam_auth_token(
        &endpoint,
        Some(&signer),
        &preset_inputs(Preset::Direct),
        mint_sources,
    )
    .await
    .unwrap();
    assert!(password.expose_secret().contains("%3F"));
    assert!(password.expose_secret().contains("%26"));
    assert!(password.expose_secret().contains("%3D"));
    assert!(password.expose_secret().contains("%2F"));
    assert!(password.expose_secret().contains("%25"));
    assert!(
        password
            .expose_secret()
            .contains("DBUser%3Duser%2540example.com%252Fweird%2520%252Bname"),
        "unexpected password: {}",
        password.expose_secret()
    );
    assert!(is_fully_escaped(password.expose_secret()));
    assert_eq!(
        percent_decode_str(password.expose_secret())
            .decode_utf8()
            .unwrap(),
        raw.expose_secret()
    );
}

fn is_fully_escaped(text: &str) -> bool {
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '%' {
            let Some(high) = characters.next() else {
                return false;
            };
            let Some(low) = characters.next() else {
                return false;
            };
            if !high.is_ascii_hexdigit() || !low.is_ascii_hexdigit() {
                return false;
            }
            continue;
        }
        if !(character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | '~')) {
            return false;
        }
    }
    true
}

#[rstest]
#[case::uniform_seconds(
    RdsTimeout::Uniform(2.5),
    Some(Duration::from_millis(2500)),
    Some(Duration::from_millis(2500))
)]
#[case::structured(RdsTimeout::ConnectRead { connect: Some(3.0_f64), read: Some(4.0_f64) }, Some(Duration::from_secs(3)), Some(Duration::from_secs(4)))]
#[case::structured_connect_only(RdsTimeout::ConnectRead { connect: Some(3.0_f64), read: None }, Some(Duration::from_secs(3)), None)]
#[case::structured_read_only(RdsTimeout::ConnectRead { connect: None, read: Some(4.0_f64) }, None, Some(Duration::from_secs(4)))]
fn float_and_structured_timeout(
    #[case] timeout: RdsTimeout,
    #[case] connect: Option<Duration>,
    #[case] read: Option<Duration>,
) {
    let config = rds_timeout_config(timeout).expect("the fixtures are valid durations");
    assert_eq!(config.connect_timeout(), connect);
    assert_eq!(config.read_timeout(), read);
}

#[test]
fn default_timeout_for_other_types() {
    let config = rds_timeout_config(RdsTimeout::SdkDefaults).expect("no duration to reject");
    assert_eq!(config, TimeoutConfig::builder().build());
    assert_eq!(config.connect_timeout(), None);
    assert_eq!(config.read_timeout(), None);
}

#[rstest]
#[case::negative(RdsTimeout::Uniform(-1.0))]
#[case::not_a_number(RdsTimeout::Uniform(f64::NAN))]
#[case::beyond_duration_seconds(RdsTimeout::ConnectRead { connect: None, read: Some(1e300_f64) })]
fn out_of_range_timeout_fails(#[case] timeout: RdsTimeout) {
    let error = rds_timeout_config(timeout).expect_err("the float is outside the duration range");
    assert!(matches!(error, RdsCredentialsError::Timeout { .. }));
}
