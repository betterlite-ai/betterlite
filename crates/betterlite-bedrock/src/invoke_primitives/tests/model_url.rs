use crate::aws_endpoints::provider::BedrockInvokeProvider;
use crate::invoke_primitives::config::Params;
use crate::invoke_primitives::url::{
    CompleteUrl, InvokeEnvironment, InvokeModelUrl, UrlError, invoke_model_url,
};

use super::support::params;

const DEFAULT_STREAMING: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-v2/invoke";

fn request<'text>(
    model: &'text str,
    api_base: Option<&'text str>,
    optional_params: &'text Params,
    stream: bool,
    environment: InvokeEnvironment<'text>,
) -> Result<InvokeModelUrl, UrlError> {
    invoke_model_url(CompleteUrl {
        api_base,
        model,
        optional_params,
        stream,
        environment,
    })
}

fn empty() -> Params {
    Params::new()
}

fn environment_with_endpoint(runtime_endpoint: Option<&str>) -> InvokeEnvironment<'_> {
    InvokeEnvironment {
        region_name: None,
        region: None,
        session_region: None,
        runtime_endpoint,
    }
}

fn plain_url(stream: bool) -> InvokeModelUrl {
    request(
        "anthropic.claude-v2",
        None,
        &empty(),
        stream,
        InvokeEnvironment::default(),
    )
    .unwrap()
}

pub(super) fn invoke_model_url_case() {
    defaults();
    api_base_wins();
    configured_and_environment();
    streaming_action();
    explicit_region();
    environment_regions();
    arn_region_and_encoding();
    invoke_prefix_stripped();
    plain_override();
    arn_override_cannot_set_region();
    malformed_parameters();
}

fn defaults() {
    let defaulted = plain_url(false);
    assert_eq!(defaulted.endpoint_url, DEFAULT_STREAMING);
    assert_eq!(defaulted.model_id, "anthropic.claude-v2");
    assert_eq!(defaulted.provider, Some(BedrockInvokeProvider::Anthropic));
    assert_eq!(defaulted.region, "us-west-2");
}

fn api_base_wins() {
    let proxy_params = params(r#"{"aws_bedrock_runtime_endpoint":"https://configured.example"}"#);
    let proxied = request(
        "anthropic.claude-v2",
        Some("https://proxy.example/"),
        &proxy_params,
        false,
        environment_with_endpoint(Some("https://environment.example")),
    )
    .unwrap();
    assert_eq!(
        proxied.endpoint_url,
        "https://proxy.example//model/anthropic.claude-v2/invoke"
    );
}

fn configured_and_environment() {
    let configured = request(
        "anthropic.claude-v2",
        None,
        &params(r#"{"aws_bedrock_runtime_endpoint":"https://configured.example"}"#),
        false,
        environment_with_endpoint(Some("https://environment.example")),
    )
    .unwrap();
    assert_eq!(
        configured.endpoint_url,
        "https://configured.example/model/anthropic.claude-v2/invoke"
    );
    let environment = request(
        "anthropic.claude-v2",
        None,
        &empty(),
        false,
        environment_with_endpoint(Some("https://environment.example")),
    )
    .unwrap();
    assert_eq!(
        environment.endpoint_url,
        "https://environment.example/model/anthropic.claude-v2/invoke"
    );
}

fn streaming_action() {
    assert_eq!(
        plain_url(true).endpoint_url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-v2/invoke-with-response-stream"
    );
    let ai21 = request(
        "ai21.j2-mid-v1",
        None,
        &empty(),
        true,
        InvokeEnvironment::default(),
    )
    .unwrap();
    assert_eq!(
        ai21.endpoint_url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/ai21.j2-mid-v1/invoke"
    );
    assert_eq!(plain_url(false).endpoint_url, DEFAULT_STREAMING);
    let unknown = request(
        "custom-model",
        None,
        &empty(),
        true,
        InvokeEnvironment::default(),
    )
    .unwrap();
    assert_eq!(
        unknown.endpoint_url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/custom-model/invoke-with-response-stream"
    );
}

fn explicit_region() {
    let explicit = request(
        "anthropic.claude-v2",
        None,
        &params(r#"{"aws_region_name":"eu-central-1"}"#),
        false,
        InvokeEnvironment {
            region_name: Some("ap-south-1"),
            region: None,
            session_region: None,
            runtime_endpoint: None,
        },
    )
    .unwrap();
    assert_eq!(explicit.region, "eu-central-1");
    assert_eq!(
        explicit.endpoint_url,
        "https://bedrock-runtime.eu-central-1.amazonaws.com/model/anthropic.claude-v2/invoke"
    );
}

fn environment_regions() {
    let from_name = request(
        "anthropic.claude-v2",
        None,
        &empty(),
        false,
        InvokeEnvironment {
            region_name: Some("ap-south-1"),
            region: Some("ca-central-1"),
            session_region: None,
            runtime_endpoint: None,
        },
    )
    .unwrap();
    assert_eq!(from_name.region, "ap-south-1");
    let from_standard = request(
        "anthropic.claude-v2",
        None,
        &empty(),
        false,
        InvokeEnvironment {
            region_name: None,
            region: Some("ca-central-1"),
            session_region: None,
            runtime_endpoint: None,
        },
    )
    .unwrap();
    assert_eq!(from_standard.region, "ca-central-1");
}

fn arn_region_and_encoding() {
    let arn = request(
        "arn:aws:bedrock:us-east-1:123456789012:inference-profile/global.anthropic.claude",
        None,
        &empty(),
        false,
        InvokeEnvironment::default(),
    )
    .unwrap();
    assert_eq!(arn.region, "us-east-1");
    assert_eq!(
        arn.model_id,
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude"
    );
    assert_eq!(
        arn.endpoint_url,
        format!(
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/{}/invoke",
            arn.model_id
        )
    );
}

fn invoke_prefix_stripped() {
    let invoke_prefixed = request(
        "invoke/anthropic.claude-v2",
        None,
        &empty(),
        false,
        InvokeEnvironment::default(),
    )
    .unwrap();
    assert_eq!(invoke_prefixed.model_id, "anthropic.claude-v2");
}

fn plain_override() {
    let overridden = request(
        "anthropic.claude-v2",
        None,
        &params(r#"{"model_id":"custom/id"}"#),
        false,
        InvokeEnvironment::default(),
    )
    .unwrap();
    assert_eq!(overridden.model_id, "custom%2Fid");
    assert_eq!(overridden.region, "us-west-2");
}

fn arn_override_cannot_set_region() {
    let overridden = request(
        "arn:aws:bedrock:eu-west-1:1:model/x",
        None,
        &params(r#"{"model_id":"arn:aws:bedrock:us-east-1:1:model/y"}"#),
        false,
        InvokeEnvironment::default(),
    )
    .unwrap();
    assert_eq!(
        overridden.model_id,
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A1%3Amodel%2Fy"
    );
    assert_eq!(overridden.region, "eu-west-1");
}

fn malformed_parameters() {
    let invalid = request(
        "anthropic.claude-v2",
        None,
        &params(r#"{"aws_region_name":"US-East-1"}"#),
        false,
        InvokeEnvironment::default(),
    );
    assert!(matches!(invalid, Err(UrlError::Endpoint(_))));
    let wrong_type = request(
        "anthropic.claude-v2",
        None,
        &params(r#"{"aws_region_name":5}"#),
        false,
        InvokeEnvironment::default(),
    );
    assert!(matches!(
        wrong_type,
        Err(UrlError::Field {
            field: "aws_region_name",
            ..
        })
    ));
}
