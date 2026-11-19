use crate::search::signing::SignRequest;
use crate::search::tests::support::{
    GATEWAY_URL, baseline, body, failing, header, lookup, now, params, secret, sign,
};

const TOKEN: &str = "super-secret-gateway-token";

#[test]
fn bedrock_services_search_secret_safe_failure() {
    assert_failures_omit_secrets();
    assert_debug_omits_secrets();
}

fn assert_failures_omit_secrets() {
    let mut case = baseline("http://plain.example.com/mcp");
    case.api_key = Some(secret(TOKEN));
    case.request_data = body(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"web-search-tool___WebSearch","arguments":{"query":"query-secret","maxResults":10}}}"#,
    );
    let error = sign(&case, &lookup(&[]), failing()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("plain.example.com"));
    assert!(!message.contains(TOKEN));
    assert!(!message.contains("query-secret"));
    assert!(!format!("{error:?}").contains(TOKEN));
    let mut rejected = baseline("https://custom.example.com/mcp");
    rejected.request_data = case.request_data;
    let environment = lookup(&[("AGENTCORE_GATEWAY_TOKEN", TOKEN)]);
    let rejected_error = sign(&rejected, &environment, failing()).unwrap_err();
    let rejected_message = rejected_error.to_string();
    assert!(rejected_message.contains("AGENTCORE_GATEWAY_TOKEN"));
    assert!(!rejected_message.contains(TOKEN));
    assert!(!format!("{rejected_error:?}").contains(TOKEN));
}

fn assert_debug_omits_secrets() {
    let mut case = baseline(GATEWAY_URL);
    case.api_key = Some(secret(TOKEN));
    let request = SignRequest {
        headers: &case.headers,
        optional_params: &case.optional_params,
        request_data: &case.request_data,
        api_base: case.api_base,
        api_key: case.api_key.as_ref(),
        session_region: case.session_region,
        now: now(),
    };
    let debug = format!("{request:?}");
    assert!(debug.contains("api_base"));
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(TOKEN));
    assert!(!debug.contains("hello"));
    let signed = sign(&case, &lookup(&[]), failing()).unwrap();
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some("Bearer super-secret-gateway-token")
    );
    let signed_debug = format!("{signed:?}");
    assert!(signed_debug.contains("Authorization"));
    assert!(!signed_debug.contains(TOKEN));
    let mut configured = baseline(GATEWAY_URL);
    configured.optional_params = params(
        r#"{"aws_access_key_id":"AKIDEXAMPLE","aws_secret_access_key":"fixture-secret-access-key"}"#,
    );
    let error = sign(&configured, &lookup(&[]), failing()).unwrap_err();
    assert!(!error.to_string().contains("fixture-secret-access-key"));
    assert!(!format!("{error:?}").contains("fixture-secret-access-key"));
}
