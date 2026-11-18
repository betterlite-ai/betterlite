use rstest::rstest;
mod integration;
use secrecy::SecretString;

use super::endpoint::{EndpointRequest, runtime_url};
use super::endpoint::{ParamMap, call_region, chat_region, endpoint_name};
use super::runtime::{
    PreparedCall, body_text, epoch_seconds, reply_headers, stream_error, wire_error,
};
use crate::aws_settings::sources::Credentials;
use crate::signing::error::SigningError;
use crate::signing::json_post::{JsonPost, sign_aws_json_post};
use betterlite_models::decisions::Json;
use futures_util::stream;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use std::time::Duration;
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

#[rstest]
#[case::completion("sagemaker/endpoint", "endpoint")]
#[case::chat("sagemaker_chat/endpoint", "endpoint")]
#[case::nova("sagemaker_nova/endpoint", "endpoint")]
#[case::already_unprefixed("endpoint", "endpoint")]
fn removes_runtime_provider_prefix(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(endpoint_name(model), expected);
}

#[test]
fn completion_region_uses_explicit_then_environment_without_session_region() {
    let params = ParamMap::new();
    assert!(matches!(
        call_region(
            &params,
            crate::signing::target::RegionInputs {
                environment_region_name: Some("eu-west-1"),
                environment_region: Some("us-east-1"),
                session_region: Some("ap-south-1"),
            }
        ),
        Ok(region) if region == "eu-west-1"
    ));
    assert!(matches!(
        call_region(&params, crate::signing::target::RegionInputs::default()),
        Ok(region) if region == "us-west-2"
    ));
    assert!(matches!(
        call_region(
            &params,
            crate::signing::target::RegionInputs {
                environment_region_name: None,
                environment_region: None,
                session_region: Some("ap-south-1"),
            }
        ),
        Ok(region) if region == "us-west-2"
    ));
}

#[test]
fn explicit_completion_region_overrides_environment() {
    let raw = serde_json::value::RawValue::from_string(String::from("\"ap-northeast-1\""))
        .expect("region JSON");
    let params = ParamMap::from([(String::from("aws_region_name"), raw)]);
    assert!(matches!(
        call_region(
            &params,
            crate::signing::target::RegionInputs {
                environment_region_name: Some("eu-west-1"),
                environment_region: Some("us-east-1"),
                session_region: None,
            }
        ),
        Ok(region) if region == "ap-northeast-1"
    ));
}

#[test]
fn chat_region_prefers_region_from_model_arn() {
    let params = ParamMap::new();
    assert!(matches!(
        chat_region(
            &params,
            "arn:aws:sagemaker:eu-central-1:123456789012:endpoint/example",
            crate::signing::target::RegionInputs {
                environment_region_name: Some("us-east-1"),
                environment_region: None,
                session_region: None,
            }
        ),
        Ok(region) if region == "eu-central-1"
    ));
}

#[test]
fn prepared_call_logging_contains_signed_request_data() {
    let prepared = PreparedCall {
        url: String::from("https://runtime.example/invocations"),
        headers: vec![(String::from("Authorization"), String::from("signature"))],
        body: br#"{"inputs":"hi"}"#.to_vec(),
        body_json: IndexMap::from([(String::from("inputs"), Json::Text(String::from("hi")))]),
        region: String::from("us-west-2"),
    };
    let logged = prepared.additional_args();
    assert_eq!(
        logged.api_base.as_deref(),
        Some("https://runtime.example/invocations")
    );
    assert!(logged.headers.is_some());
    assert!(logged.complete_input_dict.is_some());
}

#[test]
fn wire_errors_and_body_text_keep_status_and_utf8_body() {
    let error = wire_error(429, b"throttled");
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "throttled");
    assert_eq!(body_text(&[0xff, 0xfe]), "2 bytes that are not UTF-8");
    assert_eq!(epoch_seconds(UNIX_EPOCH - Duration::from_secs(1)), 0);
    assert_eq!(epoch_seconds(UNIX_EPOCH + Duration::from_secs(12)), 12);
}

#[test]
fn reply_headers_keep_valid_values_and_drop_invalid_text() {
    let mut headers = HeaderMap::new();
    headers.insert("x-result", HeaderValue::from_static("ok"));
    headers.insert("x-invalid", HeaderValue::from_bytes(&[0xff]).unwrap());
    assert_eq!(
        reply_headers(&headers).get("x-result").map(String::as_str),
        Some("ok")
    );
    assert!(!reply_headers(&headers).contains_key("x-invalid"));
}

#[tokio::test]
async fn stream_error_reads_only_the_bounded_error_body() {
    let stream = stream::iter(vec![Ok(vec![b'x'; 70_000])]);
    let chunks: super::runtime::ChunkStream = Box::pin(stream);
    let error = stream_error(503, chunks).await;
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message.len(), 65_536);
}

#[tokio::test]
async fn stream_error_truncates_the_chunk_crossing_the_body_limit() {
    let chunks = stream::iter(vec![Ok(vec![b'a'; 32_768]), Ok(vec![b'b'; 40_960])]);
    let chunks: super::runtime::ChunkStream = Box::pin(chunks);
    let error = stream_error(503, chunks).await;
    assert_eq!(error.message.len(), 65_536);
    assert!(error.message.as_bytes().ends_with(&vec![b'b'; 32_768]));
}
