#![cfg(test)]

use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tracing::subscriber::set_default;

use super::fixture::{
    Call, Clock, LogCapture, Preset, RecordingSecrets, Reply, SECRET_KEY, SIGNER_CREDENTIALS,
    aws_sources, botocore_fixture, endpoint, environment, preset_inputs, secret,
};
use crate::rds_credentials::error::RdsCredentialsError;
use crate::rds_credentials::{RdsCredentialInputs, RdsSigner, RdsTimeout, generate_iam_auth_token};

const FIXED: u64 = 1_794_225_600;

async fn mint(
    inputs: RdsCredentialInputs,
    answers: &[(&str, Option<&str>)],
) -> Result<(String, Vec<Call>, Vec<String>), RdsCredentialsError> {
    let files = [("/token", "oidc-token-value")];
    mint_against(inputs, answers, &[], &files, Reply::WebIdentity).await
}

async fn mint_against(
    inputs: RdsCredentialInputs,
    answers: &[(&str, Option<&str>)],
    environment_pairs: &[(&str, &str)],
    files: &[(&str, &str)],
    reply: Reply,
) -> Result<(String, Vec<Call>, Vec<String>), RdsCredentialsError> {
    let secrets = RecordingSecrets::new(answers);
    let mut environment = environment(&SIGNER_CREDENTIALS);
    for (name, value) in environment_pairs {
        environment.insert((*name).to_owned(), (*value).to_owned());
    }
    let clock = Clock::at(FIXED);
    let (sources, recorder) = aws_sources(&secrets, &environment, files, &clock, reply);
    let signer = RdsSigner::init(inputs, sources).await?;
    let token = signer.db_auth_token(&endpoint("db.example.com", "5432", "admin"))?;
    Ok((
        token.expose_secret().to_owned(),
        recorder.calls(),
        secrets.calls(),
    ))
}

fn region_of(token: &str) -> String {
    let scope = token
        .split("X-Amz-Credential=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("the token carries a credential scope");
    let decoded = percent_decode_str(scope)
        .decode_utf8()
        .expect("the credential scope is UTF-8");
    decoded
        .split('/')
        .nth(2)
        .expect("the credential scope names the region")
        .to_owned()
}

async fn minted_region(region_name: Option<&str>, answers: &[(&str, Option<&str>)]) -> String {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.region_name = region_name.map(str::to_owned);
    let (token, _, _) = mint(inputs, answers).await.expect("a region resolves");
    region_of(&token)
}

#[rstest]
#[case::explicit_wins(Some("eu-west-1"), &[("AWS_REGION_NAME", Some("us-east-1")), ("AWS_REGION", Some("ap-south-1"))], "eu-west-1")]
#[case::custom_before_standard(None, &[("AWS_REGION_NAME", Some("us-east-1")), ("AWS_REGION", Some("ap-south-1"))], "us-east-1")]
#[case::standard_last(None, &[("AWS_REGION_NAME", None), ("AWS_REGION", Some("ap-south-1"))], "ap-south-1")]
#[case::empty_explicit_falls_through(Some(""), &[("AWS_REGION_NAME", Some("us-east-1")), ("AWS_REGION", None)], "us-east-1")]
#[case::empty_custom_falls_through(None, &[("AWS_REGION_NAME", Some("")), ("AWS_REGION", Some("ap-south-1"))], "ap-south-1")]
#[tokio::test]
async fn explicit_region_then_custom_then_standard(
    #[case] region_name: Option<&str>,
    #[case] answers: &[(&str, Option<&str>)],
    #[case] expected: &str,
) {
    assert_eq!(minted_region(region_name, answers).await, expected);
}

#[tokio::test]
async fn region_lookup_before_parameter_secrets() {
    let answers = [
        ("AWS_REGION_NAME", Some("us-east-1")),
        ("AWS_REGION", None),
        ("REGION_REF", Some("eu-west-1")),
        ("KEY_REF", Some("AKIDREF")),
        ("SECRET_REF", Some("resolvedsecret")),
    ];
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.region_name = Some("os.environ/REGION_REF".to_owned());
    inputs.access_key_id = Some(secret("os.environ/KEY_REF"));
    inputs.secret_access_key = Some(secret("os.environ/SECRET_REF"));
    let (token, _, calls) = mint(inputs, &answers)
        .await
        .expect("the references resolve");
    let expected = [
        "AWS_REGION_NAME",
        "AWS_REGION",
        "KEY_REF",
        "SECRET_REF",
        "REGION_REF",
    ]
    .map(str::to_owned);
    assert_eq!(calls, expected.to_vec());
    assert_eq!(region_of(&token), "eu-west-1");
    assert!(token.contains("X-Amz-Credential=AKIDREF%2F"));
}

#[tokio::test]
async fn missing_region_before_credentials() {
    let secrets = RecordingSecrets::new(&[("AWS_REGION_NAME", None), ("AWS_REGION", None)]);
    let environment = environment(&[]);
    let clock = Clock::at(FIXED);
    let (sources, recorder) = aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.region_name = None;
    inputs.web_identity_token = Some(secret("oidc-secret-name"));
    let error = RdsSigner::init(inputs, sources)
        .await
        .expect_err("the region is required");
    assert_eq!(
        error.to_string(),
        "AWS region not set: set AWS_REGION_NAME or AWS_REGION env variable or in .env file"
    );
    assert!(recorder.calls().is_empty());
    assert_eq!(
        secrets.calls(),
        vec!["AWS_REGION_NAME".to_owned(), "AWS_REGION".to_owned()]
    );
}

async fn resolved_access_key() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.access_key_id = Some(secret("os.environ/KEY_REF"));
    let (token, _, _) = mint(inputs, &[("KEY_REF", Some("AKIDREF"))])
        .await
        .expect("the access key resolves");
    assert!(token.contains("X-Amz-Credential=AKIDREF%2F"));
}

async fn resolved_secret_key(direct: &str) {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.secret_access_key = Some(secret("os.environ/SECRET_REF"));
    let (token, _, _) = mint(inputs, &[("SECRET_REF", Some(SECRET_KEY))])
        .await
        .expect("the secret key resolves");
    assert_eq!(token, direct);
}

async fn resolved_region() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.region_name = Some("os.environ/REGION_REF".to_owned());
    let (token, _, _) = mint(inputs, &[("REGION_REF", Some("eu-west-1"))])
        .await
        .expect("the region reference resolves");
    assert_eq!(region_of(&token), "eu-west-1");
}

async fn resolved_sts_body(inputs: RdsCredentialInputs, name: &str, value: &str) -> String {
    let (_, calls, _) = mint(inputs, &[(name, Some(value))])
        .await
        .expect("the STS reference resolves");
    calls[0].body.clone()
}

async fn resolved_profile() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.profile_name = Some("os.environ/PROFILE_REF".to_owned());
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    let credentials = "[chosen]\naws_access_key_id = profile\naws_secret_access_key = private\n";
    let environment_pairs = [("AWS_SHARED_CREDENTIALS_FILE", "/credentials")];
    let (token, calls, _) = mint_against(
        inputs,
        &[("PROFILE_REF", Some("chosen"))],
        &environment_pairs,
        &[("/credentials", credentials)],
        Reply::WebIdentity,
    )
    .await
    .expect("the profile reference resolves");
    assert!(token.contains("X-Amz-Credential=profile%2F"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn seven_parameter_secret_references() {
    let pinned = botocore_fixture();
    let direct = pinned
        .tokens
        .iter()
        .find(|vector| vector.name == "direct_credentials")
        .expect("the direct vector");
    resolved_access_key().await;
    resolved_secret_key(&direct.token).await;
    resolved_region().await;

    let mut session_inputs = preset_inputs(Preset::WebIdentity);
    session_inputs.session_name = Some("os.environ/SESSION_REF".to_owned());
    let session_body = resolved_sts_body(session_inputs, "SESSION_REF", "session-name").await;
    assert!(session_body.contains("RoleSessionName=session-name"));

    let mut role_inputs = preset_inputs(Preset::WebIdentity);
    role_inputs.role_name = Some("os.environ/ROLE_REF".to_owned());
    let role_body = resolved_sts_body(
        role_inputs,
        "ROLE_REF",
        "arn:aws:iam::123456789012:role/oidc",
    )
    .await;
    assert!(role_body.contains("RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2Foidc"));

    let mut token_inputs = preset_inputs(Preset::WebIdentity);
    token_inputs.web_identity_token = Some(secret("os.environ/TOKEN_REF"));
    let token_body = resolved_sts_body(token_inputs, "TOKEN_REF", "/token").await;
    assert!(token_body.contains("WebIdentityToken=oidc-token-value"));

    resolved_profile().await;
}

#[tokio::test]
async fn direct_keys_before_profile() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.profile_name = Some("chosen".to_owned());
    let (token, calls, _) = mint_against(
        inputs,
        &[],
        &[("AWS_SHARED_CREDENTIALS_FILE", "/credentials")],
        &[(
            "/credentials",
            "[chosen]\naws_access_key_id = profile\naws_secret_access_key = private\n",
        )],
        Reply::WebIdentity,
    )
    .await
    .expect("the direct keys win");
    assert!(token.contains("X-Amz-Credential=AKIDEXAMPLE%2F"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn empty_direct_key_not_ambient() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.access_key_id = Some(secret(""));
    let (token, calls, _) = mint_against(
        inputs,
        &[],
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDAMBIENT"),
            ("AWS_SECRET_ACCESS_KEY", "ambientsecret"),
        ],
        &[],
        Reply::WebIdentity,
    )
    .await
    .expect("the empty direct key still signs");
    assert!(token.contains("X-Amz-Credential=%2F20261109%2F"));
    assert!(!token.contains("AKIDAMBIENT"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn profile_before_ambient() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    inputs.profile_name = Some("chosen".to_owned());
    let (token, calls, _) = mint_against(
        inputs,
        &[],
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDAMBIENT"),
            ("AWS_SECRET_ACCESS_KEY", "ambientsecret"),
            ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
        ],
        &[(
            "/credentials",
            "[chosen]\naws_access_key_id = profile\naws_secret_access_key = private\n",
        )],
        Reply::WebIdentity,
    )
    .await
    .expect("the profile outranks the ambient keys");
    assert!(token.contains("X-Amz-Credential=profile%2F"));
    assert!(!token.contains("AKIDAMBIENT"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn ambient_chain_when_nothing_is_configured() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    let (token, _, _) = mint_against(
        inputs,
        &[],
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDAMBIENT"),
            ("AWS_SECRET_ACCESS_KEY", "ambientsecret"),
        ],
        &[],
        Reply::WebIdentity,
    )
    .await
    .expect("the ambient chain provides credentials");
    assert!(token.contains("X-Amz-Credential=AKIDAMBIENT%2F"));
}

#[tokio::test]
async fn role_without_session_falls_through() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.role_name = Some("arn:aws:iam::123456789012:role/db".to_owned());
    let (token, calls, _) = mint(inputs, &[])
        .await
        .expect("the direct branch still applies");
    assert!(token.contains("X-Amz-Credential=AKIDEXAMPLE%2F"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn incomplete_direct_credentials_fail() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.secret_access_key = None;
    let error = mint(inputs, &[])
        .await
        .expect_err("a key without its secret fails");
    assert!(matches!(
        error,
        RdsCredentialsError::PartialCredentials {
            missing: "aws_secret_access_key"
        }
    ));
}

#[tokio::test]
async fn secret_lookup_failure_propagates() {
    let secrets = RecordingSecrets::failing();
    let environment = environment(&[]);
    let clock = Clock::at(FIXED);
    let (sources, recorder) = aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let error = RdsSigner::init(preset_inputs(Preset::Direct), sources)
        .await
        .expect_err("the failing lookup stops the signer");
    assert!(matches!(error, RdsCredentialsError::Secret { .. }));
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn injected_client_bypasses_env() {
    let secrets = RecordingSecrets::new(&[]);
    let environment = environment(&[]);
    let clock = Clock::at(FIXED);
    let (signer_sources, signer_recorder) =
        aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let signer = RdsSigner::init(preset_inputs(Preset::Direct), signer_sources)
        .await
        .unwrap();
    let failing = RecordingSecrets::failing();
    let empty = BTreeMap::new();
    let (mint_sources, transport) =
        aws_sources(&failing, &empty, &[], &clock, Reply::Status(500, ""));
    let unusable = RdsCredentialInputs {
        access_key_id: None,
        secret_access_key: None,
        region_name: None,
        session_name: None,
        profile_name: None,
        role_name: None,
        web_identity_token: None,
        timeout: RdsTimeout::default(),
    };
    let token = generate_iam_auth_token(
        &endpoint("db.example.com", "5432", "admin"),
        Some(&signer),
        &unusable,
        mint_sources,
    )
    .await
    .expect("the supplied client mints without the environment");
    assert!(
        token
            .expose_secret()
            .starts_with("db.example.com%3A5432%2F")
    );
    assert!(failing.calls().is_empty());
    assert!(transport.calls().is_empty());
    assert_eq!(signer_recorder.calls().len(), 0);
}

#[tokio::test]
async fn secrets_absent_from_logs() {
    let logs = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .finish();
    let _guard = set_default(subscriber);
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.web_identity_token = Some(secret("os.environ/TOKEN_REF"));
    let secrets = RecordingSecrets::new(&[("TOKEN_REF", Some("/token"))]);
    let environment = environment(&SIGNER_CREDENTIALS);
    let clock = Clock::at(FIXED);
    let (log_sources, _log_recorder) = aws_sources(
        &secrets,
        &environment,
        &[("/token", "oidc-marker-token")],
        &clock,
        Reply::WebIdentity,
    );
    let signer = RdsSigner::init(inputs.clone(), log_sources).await.unwrap();
    let token = signer
        .db_auth_token(&endpoint("db.example.com", "5432", "admin"))
        .unwrap();
    let rendered = logs.contents();
    assert!(!rendered.contains("oidc-marker-token"));
    assert!(!rendered.contains("websecret"));
    assert!(!rendered.contains("webtoken"));
    assert!(!rendered.contains("ASIAWEB"));
    assert!(!rendered.contains(SECRET_KEY));
    assert!(!rendered.contains(token.expose_secret()));
    assert!(!format!("{inputs:?}").contains("oidc-marker-token"));
}

#[tokio::test]
async fn failed_signer_text_hides_the_secrets() {
    let error = mint_against(
        preset_inputs(Preset::WebIdentity),
        &[],
        &[],
        &[("/token", "oidc-marker-token")],
        Reply::Status(
            403,
            "<ErrorResponse><Error><Code>AccessDenied</Code></Error></ErrorResponse>",
        ),
    )
    .await
    .expect_err("a refused signer fails");
    let text = format!("{error} {error:?}");
    assert!(!text.contains(SECRET_KEY));
    assert!(!text.contains("oidc-marker-token"));
    assert!(!text.contains("websecret"));
}
