use rstest::rstest;

use super::endpoint::{EndpointRequest, runtime_url};

#[rstest]
#[case::regional_invocations(
    "model",
    "us-east-1",
    None,
    false,
    false,
    "https://runtime.sagemaker.us-east-1.amazonaws.com/endpoints/model/invocations"
)]
#[case::regional_stream(
    "sagemaker_chat/model",
    "us-east-1",
    None,
    true,
    true,
    "https://runtime.sagemaker.us-east-1.amazonaws.com/endpoints/model/invocations-response-stream"
)]
#[case::china_suffix(
    "model",
    "cn-north-1",
    None,
    false,
    false,
    "https://runtime.sagemaker.cn-north-1.amazonaws.com.cn/endpoints/model/invocations"
)]
#[case::chat_base_url_adds_path(
    "model",
    "us-east-1",
    Some("https://custom.example/runtime/"),
    true,
    true,
    "https://custom.example/runtime/invocations-response-stream"
)]
#[case::chat_base_url_keeps_existing_path(
    "model",
    "us-east-1",
    Some("https://custom.example/runtime/invocations"),
    false,
    true,
    "https://custom.example/runtime/invocations"
)]
#[case::completion_base_url_replaces_url(
    "model",
    "us-east-1",
    Some("https://custom.example/custom"),
    false,
    false,
    "https://custom.example/custom"
)]
fn builds_runtime_endpoint(
    #[case] model: &str,
    #[case] region: &str,
    #[case] base_url: Option<&str>,
    #[case] stream: bool,
    #[case] append_path_to_base: bool,
    #[case] expected: &str,
) {
    assert_eq!(
        runtime_url(&EndpointRequest {
            model,
            region,
            base_url,
            stream,
            append_path_to_base,
        }),
        expected
    );
}
