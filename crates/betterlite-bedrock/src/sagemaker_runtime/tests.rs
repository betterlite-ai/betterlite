use rstest::rstest;
use secrecy::SecretString;

use super::endpoint::{EndpointRequest, runtime_url};
use crate::aws_settings::sources::Credentials;
use crate::signing::error::SigningError;
use crate::signing::json_post::{JsonPost, sign_aws_json_post};
use std::time::UNIX_EPOCH;

#[rstest]
#[case::regional_invocations(
    "model",
    "us-east-1",
    None,
    false,
    "https://runtime.sagemaker.us-east-1.amazonaws.com/endpoints/model/invocations"
)]
#[case::regional_stream(
    "sagemaker_chat/model",
    "us-east-1",
    None,
    true,
    "https://runtime.sagemaker.us-east-1.amazonaws.com/endpoints/model/invocations-response-stream"
)]
#[case::china_suffix(
    "model",
    "cn-north-1",
    None,
    false,
    "https://runtime.sagemaker.cn-north-1.amazonaws.com.cn/endpoints/model/invocations"
)]
#[case::chat_base_url_replaces_url(
    "model",
    "us-east-1",
    Some("https://custom.example/runtime/"),
    true,
    "https://custom.example/runtime/"
)]
#[case::chat_base_url_keeps_existing_path(
    "model",
    "us-east-1",
    Some("https://custom.example/runtime/invocations"),
    false,
    "https://custom.example/runtime/invocations"
)]
#[case::completion_base_url_replaces_url(
    "model",
    "us-east-1",
    Some("https://custom.example/custom"),
    false,
    "https://custom.example/custom"
)]
fn builds_runtime_endpoint(
    #[case] model: &str,
    #[case] region: &str,
    #[case] base_url: Option<&str>,
    #[case] stream: bool,
    #[case] expected: &str,
) {
    assert_eq!(
        runtime_url(&EndpointRequest {
            model,
            region,
            base_url,
            stream,
        }),
        expected
    );
}

#[test]
fn signed_post_preserves_the_final_body_bytes() -> Result<(), SigningError> {
    let credentials = Credentials {
        access_key_id: SecretString::from("access".to_owned()),
        secret_access_key: SecretString::from("secret".to_owned()),
        session_token: None,
    };
    let body = "{\"inputs\": \"hello\"}";
    let signed = sign_aws_json_post(JsonPost {
        service_name: "sagemaker",
        region: Some("us-west-2"),
        url: "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/model/invocations",
        body,
        headers: &[(
            String::from("Content-Type"),
            String::from("application/json"),
        )],
        credentials: &credentials,
        now: UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
    })?;

    assert_eq!(signed.body, body.as_bytes());
    assert!(
        signed
            .headers
            .iter()
            .any(|(name, _)| name == "Authorization")
    );
    Ok(())
}
