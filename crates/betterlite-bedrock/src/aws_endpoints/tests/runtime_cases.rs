use crate::aws_endpoints::runtime::{
    BedrockEndpointType, RuntimeEndpoint, RuntimeEndpointInput, default_endpoint_url,
    runtime_endpoint,
};

fn endpoint(
    api_base: Option<&str>,
    configured: Option<&str>,
    environment: Option<&str>,
    region: &str,
) -> RuntimeEndpoint {
    runtime_endpoint(RuntimeEndpointInput {
        api_base,
        configured_endpoint: configured,
        environment_endpoint: environment,
        region,
        endpoint_type: BedrockEndpointType::Runtime,
    })
}

pub(super) fn china_partition() {
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Runtime, "cn-north-1"),
        "https://bedrock-runtime.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Agent, "cn-northwest-1"),
        "https://bedrock-agent-runtime.cn-northwest-1.amazonaws.com.cn"
    );
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Agentcore, "cn-north-1"),
        "https://bedrock-agentcore.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Runtime, "us-gov-west-1"),
        "https://bedrock-runtime.us-gov-west-1.amazonaws.com"
    );
}

pub(super) fn runtime_agent_agentcore_endpoints() {
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Runtime, "us-east-1"),
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Agent, "us-east-1"),
        "https://bedrock-agent-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::Agentcore, "eu-west-1"),
        "https://bedrock-agentcore.eu-west-1.amazonaws.com"
    );
    assert_eq!(
        default_endpoint_url(BedrockEndpointType::default(), "us-east-1"),
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
}

pub(super) fn runtime_endpoint_precedence() {
    destination_precedence();
    environment_precedence();
    default_region_precedence();
}

fn destination_precedence() {
    let api_only = endpoint(Some("https://api.example"), None, None, "us-east-1");
    assert_eq!(api_only.endpoint_url, "https://api.example");
    assert_eq!(api_only.proxy_endpoint_url, "https://api.example");

    let configured_only = endpoint(None, Some("https://runtime.example"), None, "us-east-1");
    assert_eq!(configured_only.endpoint_url, "https://runtime.example");
    assert_eq!(
        configured_only.proxy_endpoint_url,
        "https://runtime.example"
    );

    let both = endpoint(
        Some("https://api.example"),
        Some("https://runtime.example"),
        None,
        "us-east-1",
    );
    assert_eq!(both.endpoint_url, "https://api.example");
    assert_eq!(both.proxy_endpoint_url, "https://runtime.example");
}

fn environment_precedence() {
    let api_with_environment = endpoint(
        Some("https://api.example"),
        None,
        Some("https://env.example"),
        "us-east-1",
    );
    assert_eq!(api_with_environment.endpoint_url, "https://api.example");
    assert_eq!(
        api_with_environment.proxy_endpoint_url,
        "https://env.example"
    );

    let environment_only = endpoint(None, None, Some("https://env.example"), "us-east-1");
    assert_eq!(environment_only.endpoint_url, "https://env.example");
    assert_eq!(environment_only.proxy_endpoint_url, "https://env.example");

    let empty_api_base = endpoint(Some(""), None, Some("https://env.example"), "us-east-1");
    assert_eq!(empty_api_base.endpoint_url, "");
    assert_eq!(empty_api_base.proxy_endpoint_url, "https://env.example");

    let empty_configured = endpoint(None, Some(""), Some("https://env.example"), "us-east-1");
    assert_eq!(empty_configured.endpoint_url, "");
    assert_eq!(empty_configured.proxy_endpoint_url, "");

    let empty_environment = endpoint(None, None, Some(""), "us-east-1");
    assert_eq!(
        empty_environment.endpoint_url,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(
        empty_environment.proxy_endpoint_url,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
}

fn default_region_precedence() {
    let china = endpoint(None, None, None, "cn-north-1");
    assert_eq!(
        china.endpoint_url,
        "https://bedrock-runtime.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        china.proxy_endpoint_url,
        "https://bedrock-runtime.cn-north-1.amazonaws.com.cn"
    );
}
