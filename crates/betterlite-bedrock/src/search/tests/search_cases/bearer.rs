use betterlite_config::sections::bedrock_services_search::{
    GATEWAY_TOKEN_VARIABLE, GATEWAY_URL_VARIABLE,
};

use crate::search::tests::support::{
    GATEWAY_URL, MCP_BODY_TEXT, authorization, baseline, body_text, failing, header, lookup,
    plaintext_refusal, secret, sign,
};

const TOKEN: &str = "gateway-token";

pub(super) fn assert_gateway_bearer_trust() {
    assert_gateway_token_on_a_gateway_host();
    assert_explicit_key_wins_over_server_token();
    assert_custom_host_needs_a_trusted_base();
    assert_untrusted_custom_host_is_refused();
    assert_spoofed_gateway_hosts_are_untrusted();
    assert_uppercase_gateway_host_matches();
}

fn assert_gateway_token_on_a_gateway_host() {
    let mut case = baseline(GATEWAY_URL);
    case.headers
        .insert("authorization".to_owned(), "Basic abc".to_owned());
    let environment = lookup(&[(GATEWAY_TOKEN_VARIABLE, TOKEN)]);
    let signed = sign(&case, &environment, failing()).unwrap();
    assert_eq!(signed.method, "POST");
    assert_eq!(signed.url, GATEWAY_URL);
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some("Bearer gateway-token")
    );
    assert_eq!(header(&signed.headers, "authorization"), Some("Basic abc"));
    assert_eq!(header(&signed.headers, "X-Amz-Date"), None);
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(body_text(&signed), MCP_BODY_TEXT);
}

fn assert_explicit_key_wins_over_server_token() {
    let mut case = baseline("https://custom.example.com/mcp");
    case.api_key = Some(secret("caller-token"));
    let environment = lookup(&[(GATEWAY_TOKEN_VARIABLE, TOKEN)]);
    let signed = sign(&case, &environment, failing()).unwrap();
    assert_eq!(authorization(&signed), "Bearer caller-token");
    let mut plaintext = baseline("http://custom.example.com/mcp");
    plaintext.api_key = Some(secret("caller-token"));
    let error = sign(&plaintext, &environment, failing()).unwrap_err();
    assert_eq!(
        error.to_string(),
        plaintext_refusal("http://custom.example.com/mcp")
    );
}

fn assert_custom_host_needs_a_trusted_base() {
    let api_base = "https://custom.example.com:8443/mcp";
    let environment = lookup(&[
        (GATEWAY_TOKEN_VARIABLE, TOKEN),
        (GATEWAY_URL_VARIABLE, "http://custom.example.com:8443/login"),
    ]);
    let signed = sign(&baseline(api_base), &environment, failing()).unwrap();
    assert_eq!(authorization(&signed), "Bearer gateway-token");
}

fn assert_untrusted_custom_host_is_refused() {
    let api_base = "https://custom.example.com:8443/mcp";
    let unset = lookup(&[(GATEWAY_TOKEN_VARIABLE, TOKEN)]);
    let error = sign(&baseline(api_base), &unset, failing()).unwrap_err();
    assert_eq!(error.to_string(), untrusted_base(api_base));
    let other_port = lookup(&[
        (GATEWAY_TOKEN_VARIABLE, TOKEN),
        (GATEWAY_URL_VARIABLE, "https://custom.example.com/mcp"),
    ]);
    let other_port_error = sign(&baseline(api_base), &other_port, failing()).unwrap_err();
    assert_eq!(other_port_error.to_string(), untrusted_base(api_base));
}

fn assert_spoofed_gateway_hosts_are_untrusted() {
    let spoofs = [
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com.cn/mcp",
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com.evil.com/mcp",
        "https://gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com@evil.com/mcp",
        "https://evil.com/gateway-abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com",
        "https://gateway-abc.gateway.bedrock-agentcore.amazonaws.com/mcp",
        "https://gateway_abc.gateway.bedrock-agentcore.us-east-1.amazonaws.com/mcp",
    ];
    let environment = lookup(&[(GATEWAY_TOKEN_VARIABLE, TOKEN)]);
    for api_base in spoofs {
        let error = sign(&baseline(api_base), &environment, failing()).unwrap_err();
        assert_eq!(error.to_string(), untrusted_base(api_base));
    }
}

fn assert_uppercase_gateway_host_matches() {
    let api_base = "https://GATEWAY-ABC.GATEWAY.BEDROCK-AGENTCORE.US-EAST-1.AMAZONAWS.COM/mcp";
    let environment = lookup(&[(GATEWAY_TOKEN_VARIABLE, TOKEN)]);
    let signed = sign(&baseline(api_base), &environment, failing()).unwrap();
    assert_eq!(authorization(&signed), "Bearer gateway-token");
}

fn untrusted_base(api_base: &str) -> String {
    format!(
        "refusing to send the server-configured AGENTCORE_GATEWAY_TOKEN to the caller-supplied \
         api_base '{api_base}'; pass an explicit api_key when overriding api_base for this search \
         provider"
    )
}
