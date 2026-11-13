#![cfg(test)]

use std::error::Error as _;

use secrecy::ExposeSecret as _;

use super::fixture::{
    Call, Clock, Preset, RecordingSecrets, Reply, SIGNER_CREDENTIALS, aws_sources,
    botocore_fixture, endpoint, environment, preset_inputs, secret,
};
use crate::rds_credentials::error::RdsCredentialsError;
use crate::rds_credentials::{RdsCredentialInputs, RdsSigner};

const FIXED: u64 = 1_794_225_600;
const ACCESS_DENIED: &str =
    "<ErrorResponse><Error><Code>AccessDenied</Code><Message>no</Message></Error></ErrorResponse>";

async fn run(
    inputs: RdsCredentialInputs,
    answers: &[(&str, Option<&str>)],
    files: &[(&str, &str)],
    environment_pairs: &[(&str, &str)],
    reply: Reply,
) -> (
    Result<RdsSigner, RdsCredentialsError>,
    Vec<Call>,
    Vec<String>,
) {
    let secrets = RecordingSecrets::new(answers);
    let mut environment = environment(&SIGNER_CREDENTIALS);
    for (name, value) in environment_pairs {
        environment.insert((*name).to_owned(), (*value).to_owned());
    }
    let clock = Clock::at(FIXED);
    let (sources, recorder) = aws_sources(&secrets, &environment, files, &clock, reply);
    let signer = RdsSigner::init(inputs, sources).await;
    (signer, recorder.calls(), secrets.calls())
}

#[tokio::test]
async fn web_identity_before_role_and_keys() {
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.access_key_id = Some(secret("AKIDDIRECT"));
    inputs.secret_access_key = Some(secret("directsecret"));
    let (signer, calls, _) = run(
        inputs,
        &[],
        &[("/token", "oidc-token-value")],
        &[],
        Reply::WebIdentity,
    )
    .await;
    signer.expect("the web identity branch builds a signer");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].body.contains("Action=AssumeRoleWithWebIdentity"));
    assert!(!calls[0].body.contains("Action=AssumeRole&"));
}

#[tokio::test]
async fn web_identity_file_before_secret() {
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.web_identity_token = Some(secret("/token"));
    let (signer, calls, secret_calls) = run(
        inputs,
        &[("/token", Some("secret-token"))],
        &[("/token", " file-token\n")],
        &[],
        Reply::WebIdentity,
    )
    .await;
    signer.expect("the token file builds a signer");
    assert!(calls[0].body.contains("WebIdentityToken=%20file-token%0A"));
    assert!(!calls[0].body.contains("secret-token"));
    assert!(!secret_calls.iter().any(|name| name == "/token"));
}

#[tokio::test]
async fn file_failure_secret_fallback() {
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.web_identity_token = Some(secret("/missing-token"));
    let (signer, calls, _) = run(
        inputs,
        &[("/missing-token", Some("secret-token"))],
        &[],
        &[],
        Reply::WebIdentity,
    )
    .await;
    signer.expect("the secret fallback builds a signer");
    assert!(calls[0].body.contains("WebIdentityToken=secret-token"));
}

#[tokio::test]
async fn missing_oidc_exact_error() {
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.web_identity_token = Some(secret("/missing-token"));
    let (signer, calls, _) = run(inputs, &[], &[], &[], Reply::WebIdentity).await;
    let error = signer.expect_err("a missing OIDC token stops the signer");
    assert_eq!(
        error.to_string(),
        "OIDC token could not be retrieved from secret manager."
    );
    assert!(calls.is_empty());
}

#[tokio::test]
async fn empty_oidc_reaches_sts() {
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.web_identity_token = Some(secret("/token"));
    let (signer, calls, _) = run(
        inputs,
        &[("/token", Some(""))],
        &[],
        &[],
        Reply::WebIdentity,
    )
    .await;
    signer.expect("an empty OIDC token still reaches STS");
    assert!(
        calls[0]
            .body
            .contains("WebIdentityToken=&DurationSeconds=3600")
    );
}

pub(super) async fn web_identity_duration() {
    let pinned = botocore_fixture();
    let request = &pinned.sts.assume_role_with_web_identity;
    let mut inputs = preset_inputs(Preset::WebIdentity);
    inputs.role_name = Some(request.role_arn.clone());
    inputs.session_name = Some(request.role_session_name.clone());
    inputs.web_identity_token = Some(secret("/token"));
    let (signer, calls, _) = run(
        inputs,
        &[],
        &[("/token", request.web_identity_token.as_str())],
        &[],
        Reply::WebIdentity,
    )
    .await;
    signer.expect("the web identity signer builds");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].uri, request.url);
    assert_eq!(calls[0].body, request.body);
    assert_eq!(request.duration_seconds, 3_600_i32);
    assert!(calls[0].body.contains("DurationSeconds=3600"));
}

#[tokio::test]
async fn web_identity_sts_uses_ambient_credentials() {
    let mut inputs = preset_inputs(Preset::WebIdentity);
    // a partial explicit pair fails the role and direct branches, so this branch must not use it
    inputs.access_key_id = Some(secret("AKIDPARAM"));
    inputs.secret_access_key = None;
    let (signer, calls, _) = run(
        inputs,
        &[],
        &[("/token", "oidc-token-value")],
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDAMBIENT"),
            ("AWS_SECRET_ACCESS_KEY", "ambientsecret"),
        ],
        Reply::WebIdentity,
    )
    .await;
    signer.expect("the web identity branch ignores the explicit pair");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].body.contains("Action=AssumeRoleWithWebIdentity"));
    // AssumeRoleWithWebIdentity is an unsigned STS operation, and botocore sends it without an Authorization header too
    assert!(calls[0].authorization.is_empty());
}

#[tokio::test]
async fn role_sts_uses_explicit_credentials() {
    let mut inputs = preset_inputs(Preset::AssumeRole);
    inputs.access_key_id = Some(secret("AKIDEXPLICIT"));
    inputs.secret_access_key = Some(secret("explicitsecret"));
    let (signer, calls, _) = run(
        inputs,
        &[],
        &[],
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDAMBIENT"),
            ("AWS_SECRET_ACCESS_KEY", "ambientsecret"),
            ("AWS_SESSION_TOKEN", "envtoken"),
        ],
        Reply::AssumeRole,
    )
    .await;
    signer.expect("the assume role signer builds");
    assert!(calls[0].authorization.contains("Credential=AKIDEXPLICIT/"));
    assert!(!calls[0].authorization.contains("AKIDAMBIENT"));
    assert!(calls[0].security_token.is_empty());
}

#[tokio::test]
async fn role_request_has_no_duration_override() {
    let pinned = botocore_fixture();
    let request = &pinned.sts.assume_role;
    let mut inputs = preset_inputs(Preset::AssumeRole);
    inputs.role_name = Some(request.role_arn.clone());
    inputs.session_name = Some(request.role_session_name.clone());
    let (signer, calls, _) = run(inputs, &[], &[], &[], Reply::AssumeRole).await;
    signer.expect("the assume role signer builds");
    assert_eq!(calls[0].uri, request.url);
    assert_eq!(calls[0].body, request.body);
    assert!(!calls[0].body.contains("DurationSeconds"));
}

#[tokio::test]
async fn temporary_session_token_reaches_rds() {
    let (web_identity, _, _) = run(
        preset_inputs(Preset::WebIdentity),
        &[],
        &[("/token", "oidc-token-value")],
        &[],
        Reply::WebIdentity,
    )
    .await;
    let web_token = web_identity
        .expect("the web identity signer builds")
        .db_auth_token(&endpoint("db.example.com", "5432", "admin"))
        .unwrap();
    assert!(
        web_token
            .expose_secret()
            .contains("X-Amz-Security-Token=webtoken")
    );
    let (assumed, _, _) = run(
        preset_inputs(Preset::AssumeRole),
        &[],
        &[],
        &[],
        Reply::AssumeRole,
    )
    .await;
    let role_token = assumed
        .expect("the assume role signer builds")
        .db_auth_token(&endpoint("db.example.com", "5432", "admin"))
        .unwrap();
    assert!(
        role_token
            .expose_secret()
            .contains("X-Amz-Security-Token=roletoken")
    );
}

#[tokio::test]
async fn upstream_failure_no_strategy_fallback() {
    let mut inputs = preset_inputs(Preset::AssumeRole);
    inputs.access_key_id = Some(secret("AKIDEXPLICIT"));
    inputs.secret_access_key = Some(secret("explicitsecret"));
    let (signer, calls, _) = run(inputs, &[], &[], &[], Reply::Status(403, ACCESS_DENIED)).await;
    let error = signer.expect_err("a refused STS call fails the signer");
    assert!(matches!(error, RdsCredentialsError::AssumeRole { .. }));
    assert!(error.source().is_some());
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn missing_sts_response_field_fails() {
    let (missing, _, _) = run(
        preset_inputs(Preset::AssumeRole),
        &[],
        &[],
        &[],
        Reply::MissingSessionToken,
    )
    .await;
    let missing_error = missing.expect_err("a response without a session token fails");
    assert!(matches!(
        missing_error,
        RdsCredentialsError::StsMissingField {
            field: "SessionToken"
        }
    ));
}

#[tokio::test]
async fn missing_sts_access_key_fails() {
    let (no_key, _, _) = run(
        preset_inputs(Preset::AssumeRole),
        &[],
        &[],
        &[],
        Reply::MissingAccessKey,
    )
    .await;
    let no_key_error = no_key.expect_err("a response without an access key fails");
    assert!(matches!(
        no_key_error,
        RdsCredentialsError::StsMissingField {
            field: "AccessKeyId"
        }
    ));
}

#[tokio::test]
async fn sts_response_without_credentials_fails() {
    let (empty, _, _) = run(
        preset_inputs(Preset::AssumeRole),
        &[],
        &[],
        &[],
        Reply::WithoutCredentials,
    )
    .await;
    let empty_error = empty.expect_err("a response without credentials fails");
    assert!(matches!(
        empty_error,
        RdsCredentialsError::StsWithoutCredentials
    ));
}

#[tokio::test]
async fn web_identity_failure_keeps_the_web_identity_error() {
    let (signer, calls, _) = run(
        preset_inputs(Preset::WebIdentity),
        &[],
        &[("/token", "oidc-token-value")],
        &[],
        Reply::Status(403, ACCESS_DENIED),
    )
    .await;
    let error = signer.expect_err("a refused STS call fails the signer");
    assert!(matches!(
        error,
        RdsCredentialsError::AssumeRoleWithWebIdentity { .. }
    ));
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn missing_profile_does_not_fall_through_to_ambient() {
    let mut inputs = preset_inputs(Preset::Direct);
    inputs.access_key_id = None;
    inputs.secret_access_key = None;
    inputs.profile_name = Some("missing".to_owned());
    let (signer, calls, _) = run(
        inputs,
        &[],
        &[
            (
                "/credentials",
                "[other]\naws_access_key_id = other\naws_secret_access_key = private\n",
            ),
            ("/token", "oidc-token-value"),
        ],
        &[
            ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
            ("AWS_ROLE_ARN", "arn:aws:iam::123456789012:role/pod"),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", "/token"),
        ],
        Reply::WebIdentity,
    )
    .await;
    let error = signer.expect_err("a missing profile fails even when the pod identity could sign");
    assert!(matches!(error, RdsCredentialsError::MissingProfile { .. }));
    assert!(error.to_string().contains("missing"));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn role_without_credentials_uses_the_ambient_chain() {
    let (signer, calls, _) = run(
        preset_inputs(Preset::AssumeRole),
        &[],
        &[],
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDAMBIENT"),
            ("AWS_SECRET_ACCESS_KEY", "ambientsecret"),
        ],
        Reply::AssumeRole,
    )
    .await;
    signer.expect("the ambient credentials sign the STS call");
    assert!(
        calls[0].authorization.contains("Credential=AKIDAMBIENT/"),
        "authorization: {}",
        calls[0].authorization
    );
}

#[tokio::test]
async fn empty_role_name_still_takes_the_role_branch() {
    let mut inputs = preset_inputs(Preset::AssumeRole);
    inputs.role_name = Some(String::new());
    let (signer, calls, _) = run(inputs, &[], &[], &[], Reply::AssumeRole).await;
    signer.expect("an empty role name still asks STS");
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .body
            .contains("RoleArn=&RoleSessionName=db-session")
    );
}

#[tokio::test]
async fn partial_role_credentials_fail_before_the_call() {
    let mut inputs = preset_inputs(Preset::AssumeRole);
    inputs.access_key_id = Some(secret("AKIDONLY"));
    let (signer, calls, _) = run(inputs, &[], &[], &[], Reply::AssumeRole).await;
    let error = signer.expect_err("a key without its secret fails");
    assert!(matches!(
        error,
        RdsCredentialsError::PartialCredentials {
            missing: "aws_secret_access_key"
        }
    ));
    assert!(calls.is_empty());
}
