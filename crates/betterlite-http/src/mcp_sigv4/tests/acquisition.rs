#![cfg(test)]

use rstest::rstest;

use super::fixture::{
    Reply, VECTOR_TIME, blank_settings, built, explicit_settings, mcp_request, secret,
};
use crate::mcp_sigv4::error::McpSigV4Error;
use crate::mcp_sigv4::{McpSigV4Auth, McpSigV4Settings};

const ENVIRONMENT_CREDENTIALS: [(&str, &str); 3] = [
    ("AWS_ACCESS_KEY_ID", "AKIDFROMENV"),
    ("AWS_SECRET_ACCESS_KEY", "envsecret"),
    ("AWS_EC2_METADATA_DISABLED", "true"),
];

async fn signed_authorization(auth: &McpSigV4Auth) -> String {
    let signed = auth.auth_flow(mcp_request("{}")).await.unwrap();
    signed.headers()["authorization"]
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn explicit_credentials_sign_without_any_network_call() {
    let (auth, recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let authorization = signed_authorization(&auth).await;
    assert!(authorization.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/bedrock-agentcore/aws4_request, "
    ));
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn explicit_credentials_with_a_session_token_send_the_security_token_header() {
    let mut settings = explicit_settings();
    settings.aws_session_token = Some(secret("session-token"));
    let (auth, _recorder, _clock) = built(settings, &[], Reply::Refused).await;
    let signed = auth.auth_flow(mcp_request("{}")).await.unwrap();
    assert_eq!(signed.headers()["x-amz-security-token"], "session-token");
    let authorization = signed.headers()["authorization"].to_str().unwrap();
    assert!(
        authorization.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token,")
    );
}

#[tokio::test]
async fn explicit_credentials_win_over_the_environment() {
    let (auth, _recorder, _clock) = built(
        explicit_settings(),
        &ENVIRONMENT_CREDENTIALS,
        Reply::Refused,
    )
    .await;
    assert!(
        signed_authorization(&auth)
            .await
            .contains("Credential=AKIDEXAMPLE/")
    );
}

#[rstest]
#[case::key_only(Some(secret("AKIDLONELY")), None)]
#[case::secret_only(None, Some(secret("lonelysecret")))]
#[tokio::test]
async fn default_chain_is_used_when_only_one_explicit_key_is_given(
    #[case] aws_access_key_id: Option<secrecy::SecretString>,
    #[case] aws_secret_access_key: Option<secrecy::SecretString>,
) {
    let settings = McpSigV4Settings {
        aws_access_key_id,
        aws_secret_access_key,
        ..blank_settings()
    };
    let (auth, _recorder, _clock) = built(settings, &ENVIRONMENT_CREDENTIALS, Reply::Refused).await;
    assert!(
        signed_authorization(&auth)
            .await
            .contains("Credential=AKIDFROMENV/")
    );
}

#[tokio::test]
async fn default_chain_reads_environment_credentials_and_the_session_token() {
    let environment = [
        ("AWS_ACCESS_KEY_ID", "AKIDFROMENV"),
        ("AWS_SECRET_ACCESS_KEY", "envsecret"),
        ("AWS_SESSION_TOKEN", "envtoken"),
    ];
    let (auth, _recorder, _clock) =
        built(McpSigV4Settings::default(), &environment, Reply::Refused).await;
    let signed = auth.auth_flow(mcp_request("{}")).await.unwrap();
    assert_eq!(signed.headers()["x-amz-security-token"], "envtoken");
}

#[tokio::test]
async fn default_chain_reads_the_shared_credentials_file() {
    let clock = super::fixture::Clock::at(VECTOR_TIME);
    let files = [(
        "/home/mcp/.aws/credentials",
        "[default]\naws_access_key_id = AKIDFROMFILE\naws_secret_access_key = filesecret\n",
    )];
    let environment = [("HOME", "/home/mcp"), ("AWS_EC2_METADATA_DISABLED", "true")];
    let (sources, _recorder) =
        super::fixture::sources(&environment, &files, &clock, Reply::Refused);
    let auth = McpSigV4Auth::new(McpSigV4Settings::default(), sources)
        .await
        .unwrap();
    assert!(
        signed_authorization(&auth)
            .await
            .contains("Credential=AKIDFROMFILE/")
    );
}

#[rstest]
#[case::explicit_access_key("aws_access_key_id")]
#[case::explicit_secret_key("aws_secret_access_key")]
#[case::sts_assume_role("AssumeRole")]
#[case::default_chain("default credentials")]
#[tokio::test]
async fn no_creds_found_is_an_error_naming_all_three_options(#[case] option: &str) {
    let clock = super::fixture::Clock::at(VECTOR_TIME);
    let environment = [("AWS_EC2_METADATA_DISABLED", "true")];
    let (sources, _recorder) = super::fixture::sources(&environment, &[], &clock, Reply::Refused);
    let error = McpSigV4Auth::new(McpSigV4Settings::default(), sources)
        .await
        .err()
        .unwrap();
    let message = error.to_string();
    assert!(matches!(error, McpSigV4Error::NoCredentials));
    assert!(message.contains(option), "{message} names {option}");
}

#[rstest]
#[case::nothing_given(McpSigV4Settings::default(), "bedrock-agentcore", "us-east-1")]
#[case::service_only(
    McpSigV4Settings { aws_service_name: Some("execute-api".to_owned()), ..explicit_settings() },
    "execute-api",
    "us-east-1"
)]
#[case::region_only(
    McpSigV4Settings { aws_region_name: Some("eu-west-1".to_owned()), ..explicit_settings() },
    "bedrock-agentcore",
    "eu-west-1"
)]
#[tokio::test]
async fn defaults_apply_to_service_and_region_when_absent(
    #[case] mut settings: McpSigV4Settings,
    #[case] service: &str,
    #[case] region: &str,
) {
    if settings.aws_access_key_id.is_none() {
        settings.aws_access_key_id = explicit_settings().aws_access_key_id;
        settings.aws_secret_access_key = explicit_settings().aws_secret_access_key;
    }
    let (auth, _recorder, _clock) = built(settings, &[], Reply::Refused).await;
    let scope = format!("/20150830/{region}/{service}/aws4_request, ");
    assert!(signed_authorization(&auth).await.contains(&scope));
}

#[tokio::test]
async fn the_signer_asks_httpx_for_the_request_body() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    assert!(auth.requires_request_body);
}

#[tokio::test]
async fn no_runtime_import_is_needed_to_construct_or_sign() {
    let (auth, recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    auth.auth_flow(mcp_request("{}")).await.unwrap();
    assert!(recorder.calls().is_empty());
}
