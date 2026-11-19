use betterlite_config::sections::bedrock_services_search::GATEWAY_URL_VARIABLE;

use crate::search::error::SearchError;
use crate::search::tests::support::{
    GATEWAY_URL, MCP_BODY_TEXT, authorization, baseline, body_text, header, lookup, params,
    resolver, sign,
};

pub(super) fn assert_gateway_host_signing() {
    assert_gateway_host_is_sigv4_signed();
    assert_configured_hostname_allows_a_custom_host();
    assert_untrusted_signing_host_is_refused();
    assert_spoofed_signing_hosts_need_configuration();
}

fn assert_gateway_host_is_sigv4_signed() {
    let signed = sign(&baseline(GATEWAY_URL), &lookup(&[]), resolver()).unwrap();
    assert_eq!(signed.method, "POST");
    assert_eq!(signed.url, GATEWAY_URL);
    let signature = authorization(&signed);
    assert!(signature.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-east-1/bedrock-agentcore/aws4_request"
    ));
    assert!(signature.contains("SignedHeaders=content-type;host;x-amz-date,"));
    assert_eq!(
        header(&signed.headers, "X-Amz-Date"),
        Some("20260101T000000Z")
    );
    assert_eq!(
        header(&signed.headers, "MCP-Protocol-Version"),
        Some("2025-03-26")
    );
    assert_eq!(body_text(&signed), MCP_BODY_TEXT);
}

fn assert_configured_hostname_allows_a_custom_host() {
    let api_base = "https://custom.example.com:8443/mcp";
    let mut case = baseline(api_base);
    case.session_region = Some("eu-west-1");
    // hostname-only comparison: scheme, port and path of the configured URL are ignored
    let environment = lookup(&[(GATEWAY_URL_VARIABLE, "http://custom.example.com/other")]);
    let signed = sign(&case, &environment, resolver()).unwrap();
    assert!(authorization(&signed).contains("/20260101/eu-west-1/bedrock-agentcore/aws4_request"));
}

fn assert_untrusted_signing_host_is_refused() {
    let api_base = "https://custom.example.com/mcp";
    let mut case = baseline(api_base);
    case.session_region = Some("eu-west-1");
    let error = sign(&case, &lookup(&[]), resolver()).unwrap_err();
    assert_eq!(error.to_string(), signing_refusal(api_base));
    let environment = lookup(&[(GATEWAY_URL_VARIABLE, "https://other.example.com/mcp")]);
    let error = sign(&case, &environment, resolver()).unwrap_err();
    assert_eq!(error.to_string(), signing_refusal(api_base));
}

fn assert_spoofed_signing_hosts_need_configuration() {
    let spoofs = [
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com.cn/mcp",
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com.evil.com/mcp",
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com@evil.com/mcp",
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.org/mcp",
        "https://gateway-abc.gateway.bedrock-agentcore..amazonaws.com/mcp",
    ];
    for api_base in spoofs {
        let mut case = baseline(api_base);
        case.session_region = Some("us-east-1");
        let error = sign(&case, &lookup(&[]), resolver()).unwrap_err();
        assert_eq!(error.to_string(), signing_refusal(api_base));
    }
}

fn signing_refusal(api_base: &str) -> String {
    format!(
        "Refusing to send SigV4-signed AgentCore requests to '{api_base}': it is neither an \
         AgentCore gateway hostname nor the host in AGENTCORE_GATEWAY_URL. Set \
         AGENTCORE_GATEWAY_URL to authorize a custom gateway hostname."
    )
}

pub(super) fn assert_gateway_signing_region() {
    assert_gateway_region_wins_over_session();
    assert_explicit_region_is_kept();
    assert_empty_explicit_region_is_invalid();
    assert_null_region_falls_through();
    assert_custom_host_uses_session_region();
    assert_missing_region_is_refused();
    assert_runtime_bearer_token_is_not_sent();
}

fn assert_gateway_region_wins_over_session() {
    let mut case = baseline(GATEWAY_URL);
    case.session_region = Some("eu-west-1");
    let signed = sign(&case, &lookup(&[]), resolver()).unwrap();
    assert!(authorization(&signed).contains("/20260101/us-east-1/bedrock-agentcore/aws4_request"));
}

fn assert_explicit_region_is_kept() {
    let mut case = baseline(GATEWAY_URL);
    case.optional_params = params(r#"{"aws_region_name":"ap-southeast-2"}"#);
    let signed = sign(&case, &lookup(&[]), resolver()).unwrap();
    assert!(
        authorization(&signed).contains("/20260101/ap-southeast-2/bedrock-agentcore/aws4_request")
    );
    assert_eq!(body_text(&signed), MCP_BODY_TEXT);
}

fn assert_empty_explicit_region_is_invalid() {
    let mut case = baseline(GATEWAY_URL);
    case.optional_params = params(r#"{"aws_region_name":""}"#);
    let error = sign(&case, &lookup(&[]), resolver()).unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, SearchError::Signing(_)));
    assert!(message.contains("invalid AWS region format"));
}

fn assert_null_region_falls_through() {
    let mut case = baseline(GATEWAY_URL);
    case.optional_params = params(r#"{"aws_region_name":null}"#);
    let signed = sign(&case, &lookup(&[]), resolver()).unwrap();
    assert!(authorization(&signed).contains("/20260101/us-east-1/bedrock-agentcore/aws4_request"));
}

fn assert_custom_host_uses_session_region() {
    let api_base = "https://custom.example.com/mcp";
    let mut case = baseline(api_base);
    case.session_region = Some("eu-west-1");
    let environment = lookup(&[(GATEWAY_URL_VARIABLE, api_base)]);
    let signed = sign(&case, &environment, resolver()).unwrap();
    assert!(authorization(&signed).contains("/20260101/eu-west-1/bedrock-agentcore/aws4_request"));
}

fn assert_missing_region_is_refused() {
    let api_base = "https://custom.example.com/mcp";
    let environment = lookup(&[(GATEWAY_URL_VARIABLE, api_base)]);
    let error = sign(&baseline(api_base), &environment, resolver()).unwrap_err();
    assert_eq!(error.to_string(), region_refusal(api_base));
    let mut blank = baseline(api_base);
    blank.session_region = Some("");
    let error = sign(&blank, &environment, resolver()).unwrap_err();
    assert_eq!(error.to_string(), region_refusal(api_base));
}

fn assert_runtime_bearer_token_is_not_sent() {
    let environment = lookup(&[("AWS_BEARER_TOKEN_BEDROCK", "bedrock-runtime-token")]);
    let signed = sign(&baseline(GATEWAY_URL), &environment, resolver()).unwrap();
    assert!(authorization(&signed).starts_with("AWS4-HMAC-SHA256 "));
    assert!(
        signed
            .headers
            .iter()
            .all(|(_, value)| !value.contains("bedrock-runtime-token"))
    );
}

fn region_refusal(api_base: &str) -> String {
    format!(
        "Cannot derive the SigV4 signing region from api_base '{api_base}' or the AWS \
         configuration chain. Set aws_region_name (or AWS_DEFAULT_REGION / a profile region) to \
         the gateway's region when using a custom hostname."
    )
}
