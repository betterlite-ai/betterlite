#![cfg(test)]

use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tracing::subscriber::set_default;

use super::fixture::{
    Call, Clock, LogCapture, RecordingSecrets, Reply, SECRET_KEY, aws_sources, botocore_fixture,
    direct_inputs, endpoint, environment, secret, web_identity_inputs,
};
use crate::rds_credentials::error::RdsCredentialsError;
use crate::rds_credentials::{RdsCredentialInputs, RdsSigner, RdsTimeout, generate_iam_auth_token};

const FIXED: u64 = 1_794_225_600;

async fn mint(
    inputs: RdsCredentialInputs,
    answers: &[(&str, Option<&str>)],
    files: &[(&str, &str)],
    reply: Reply,
) -> Result<(String, Vec<Call>, Vec<String>), RdsCredentialsError> {
    let secrets = RecordingSecrets::new(answers);
    let environment = environment(&[]);
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
    let mut inputs = direct_inputs();
    inputs.region_name = region_name.map(str::to_owned);
    let (token, _, _) = mint(inputs, answers, &[], Reply::WebIdentity)
        .await
        .expect("a region resolves");
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
    let mut inputs = direct_inputs();
    inputs.region_name = Some("os.environ/REGION_REF".to_owned());
    inputs.access_key_id = Some(secret("os.environ/KEY_REF"));
    inputs.secret_access_key = Some(secret("os.environ/SECRET_REF"));
    let (token, _, calls) = mint(inputs, &answers, &[], Reply::WebIdentity)
        .await
        .expect("the references resolve");
    assert_eq!(calls.first().map(String::as_str), Some("AWS_REGION_NAME"));
    assert_eq!(calls.get(1).map(String::as_str), Some("AWS_REGION"));
    let region_position = calls.iter().position(|name| name == "REGION_REF").unwrap();
    let key_position = calls.iter().position(|name| name == "KEY_REF").unwrap();
    assert!(region_position > 1 && key_position > region_position);
    assert_eq!(region_of(&token), "eu-west-1");
    assert!(token.contains("X-Amz-Credential=AKIDREF%2F"));
}

#[tokio::test]
async fn missing_region_before_credentials() {
    let secrets = RecordingSecrets::new(&[("AWS_REGION_NAME", None), ("AWS_REGION", None)]);
    let environment = environment(&[]);
    let clock = Clock::at(FIXED);
    let (sources, recorder) = aws_sources(&secrets, &environment, &[], &clock, Reply::WebIdentity);
    let mut inputs = web_identity_inputs();
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

async fn resolve_access_key_reference() {
    let mut inputs = direct_inputs();
    inputs.region_name = None;
    inputs.access_key_id = Some(secret("os.environ/KEY_REF"));
    let (token, _, _) = mint(
        inputs,
        &[
            ("AWS_REGION_NAME", Some("us-east-1")),
            ("KEY_REF", Some("AKIDREF")),
        ],
        &[],
        Reply::WebIdentity,
    )
    .await
    .expect("the access key resolves");
    assert!(token.contains("X-Amz-Credential=AKIDREF%2F"));
}

async fn resolve_secret_key_reference(direct: &str) {
    let mut inputs = direct_inputs();
    inputs.secret_access_key = Some(secret("os.environ/SECRET_REF"));
    let (token, _, _) = mint(
        inputs,
        &[("SECRET_REF", Some(SECRET_KEY))],
        &[],
        Reply::WebIdentity,
    )
    .await
    .expect("the secret key resolves");
    assert_eq!(token, direct);
    assert_eq!(region_of(&token), "us-east-1");
}

async fn resolve_region_reference() {
    let mut inputs = direct_inputs();
    inputs.region_name = Some("os.environ/REGION_REF".to_owned());
    let (token, _, _) = mint(
        inputs,
        &[("REGION_REF", Some("eu-west-1"))],
        &[],
        Reply::WebIdentity,
    )
    .await
    .expect("the region reference resolves");
    assert_eq!(region_of(&token), "eu-west-1");
}

async fn resolve_session_name_reference() {
    let mut inputs = web_identity_inputs();
    inputs.session_name = Some("os.environ/SESSION_REF".to_owned());
    let (_, calls, _) = mint(
        inputs,
        &[("SESSION_REF", Some("session-name"))],
        &[("/token", "oidc-token-value")],
        Reply::WebIdentity,
    )
    .await
    .expect("the session name resolves");
    assert!(calls[0].body.contains("RoleSessionName=session-name"));
}

async fn resolve_role_name_reference() {
    let mut inputs = web_identity_inputs();
    inputs.role_name = Some("os.environ/ROLE_REF".to_owned());
    let (_, calls, _) = mint(
        inputs,
        &[("ROLE_REF", Some("arn:aws:iam::123456789012:role/oidc"))],
        &[("/token", "oidc-token-value")],
        Reply::WebIdentity,
    )
    .await
    .expect("the role name resolves");
    assert!(
        calls[0]
            .body
            .contains("RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2Foidc")
    );
}

async fn resolve_web_identity_token_reference() {
    let mut inputs = web_identity_inputs();
    inputs.web_identity_token = Some(secret("os.environ/TOKEN_REF"));
    let (_, calls, _) = mint(
        inputs,
        &[("TOKEN_REF", Some("/token"))],
        &[("/token", "oidc-token-value")],
        Reply::WebIdentity,
    )
    .await
    .expect("the web identity token reference resolves");
    assert!(calls[0].body.contains("WebIdentityToken=oidc-token-value"));
}

async fn resolve_profile_reference() {
    let mut inputs = direct_inputs();
    inputs.profile_name = Some("os.environ/PROFILE_REF".to_owned());
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    let (token, calls, _) = mint(
        inputs,
        &[
            ("PROFILE_REF", Some("chosen")),
            ("AWS_SHARED_CREDENTIALS_FILE", Some("/credentials")),
            ("AWS_EC2_METADATA_DISABLED", Some("true")),
        ],
        &[(
            "/credentials",
            "[chosen]\naws_access_key_id = profile\naws_secret_access_key = private\n",
        )],
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
    resolve_access_key_reference().await;
    resolve_secret_key_reference(&direct.token).await;
    resolve_region_reference().await;
    resolve_session_name_reference().await;
    resolve_role_name_reference().await;
    resolve_web_identity_token_reference().await;
    resolve_profile_reference().await;
}

#[tokio::test]
async fn direct_keys_before_profile() {
    let mut inputs = direct_inputs();
    inputs.profile_name = Some("chosen".to_owned());
    let (token, calls, _) = mint(
        inputs,
        &[
            ("AWS_SHARED_CREDENTIALS_FILE", Some("/credentials")),
            ("AWS_EC2_METADATA_DISABLED", Some("true")),
        ],
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
    let mut inputs = direct_inputs();
    inputs.access_key_id = Some(secret(""));
    let (token, calls, _) = mint(
        inputs,
        &[
            ("AWS_ACCESS_KEY_ID", Some("AKIDAMBIENT")),
            ("AWS_SECRET_ACCESS_KEY", Some("ambientsecret")),
            ("AWS_EC2_METADATA_DISABLED", Some("true")),
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
    let mut inputs = direct_inputs();
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    inputs.profile_name = Some("chosen".to_owned());
    let (token, calls, _) = mint(
        inputs,
        &[
            ("AWS_ACCESS_KEY_ID", Some("AKIDAMBIENT")),
            ("AWS_SECRET_ACCESS_KEY", Some("ambientsecret")),
            ("AWS_SHARED_CREDENTIALS_FILE", Some("/credentials")),
            ("AWS_EC2_METADATA_DISABLED", Some("true")),
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
    let mut inputs = direct_inputs();
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    let (token, _, _) = mint(
        inputs,
        &[
            ("AWS_ACCESS_KEY_ID", Some("AKIDAMBIENT")),
            ("AWS_SECRET_ACCESS_KEY", Some("ambientsecret")),
            ("AWS_EC2_METADATA_DISABLED", Some("true")),
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
    let mut inputs = direct_inputs();
    inputs.role_name = Some("arn:aws:iam::123456789012:role/db".to_owned());
    let (token, calls, _) = mint(inputs, &[], &[], Reply::WebIdentity)
        .await
        .expect("the direct branch still applies");
    assert!(token.contains("X-Amz-Credential=AKIDEXAMPLE%2F"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn incomplete_direct_credentials_fail() {
    let mut inputs = direct_inputs();
    inputs.secret_access_key = None;
    let error = mint(inputs, &[], &[], Reply::WebIdentity)
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
    let error = RdsSigner::init(direct_inputs(), sources)
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
    let signer = RdsSigner::init(direct_inputs(), signer_sources)
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
    let mut inputs = web_identity_inputs();
    inputs.web_identity_token = Some(secret("os.environ/TOKEN_REF"));
    let secrets = RecordingSecrets::new(&[("TOKEN_REF", Some("/token"))]);
    let environment = environment(&[]);
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
    let error = mint(
        web_identity_inputs(),
        &[("AWS_REGION_NAME", Some("us-east-1"))],
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
