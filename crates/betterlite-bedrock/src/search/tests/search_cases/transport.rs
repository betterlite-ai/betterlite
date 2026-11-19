use crate::search::tests::support::{
    GATEWAY_URL, baseline, body, failing, header, lookup, plaintext_refusal, resolver, secret, sign,
};

pub(super) fn assert_https_or_loopback_only() {
    assert_non_object_body_is_refused_first();
    assert_plaintext_remote_host_is_refused();
    assert_https_is_accepted();
    assert_loopback_hosts_are_accepted_on_any_scheme();
    assert_loopback_match_is_exact();
    assert_unparseable_base_is_refused();
}

fn assert_non_object_body_is_refused_first() {
    let mut case = baseline("http://plain.example.com/mcp");
    case.request_data = body(r#"[{"jsonrpc":"2.0","id":1}]"#);
    let error = sign(&case, &lookup(&[]), failing()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "AgentCore search expects a single dict request body"
    );
}

fn assert_plaintext_remote_host_is_refused() {
    let api_base = "http://gateway-abc123.gateway.bedrock-agentcore.us-east-1.amazonaws.com/mcp";
    let error = sign(&baseline(api_base), &lookup(&[]), failing()).unwrap_err();
    assert_eq!(error.to_string(), plaintext_refusal(api_base));
    // the guard runs before credential lookup, so even an explicit caller key is refused
    let mut keyed = baseline(api_base);
    keyed.api_key = Some(secret("caller-token"));
    let error = sign(&keyed, &lookup(&[]), failing()).unwrap_err();
    assert_eq!(error.to_string(), plaintext_refusal(api_base));
}

fn assert_https_is_accepted() {
    let signed = sign(&baseline(GATEWAY_URL), &lookup(&[]), resolver()).unwrap();
    assert_eq!(signed.url, GATEWAY_URL);
}

fn assert_loopback_hosts_are_accepted_on_any_scheme() {
    let accepted = [
        "http://localhost/mcp",
        "http://LOCALHOST/mcp",
        "http://127.0.0.1:8080/mcp",
        "http://[::1]:8080/mcp",
        "ftp://localhost/mcp",
    ];
    for api_base in accepted {
        let mut case = baseline(api_base);
        case.api_key = Some(secret("caller-token"));
        let signed = sign(&case, &lookup(&[]), failing()).unwrap();
        assert_eq!(
            header(&signed.headers, "Authorization"),
            Some("Bearer caller-token")
        );
        assert_eq!(signed.url, api_base);
    }
}

fn assert_loopback_match_is_exact() {
    let refused = [
        "http://127.0.0.2/mcp",
        "http://[::2]/mcp",
        "http://localhost.evil.com/mcp",
        "http://localhost@evil.com/mcp",
        "http://localhost:8080@evil.com/mcp",
    ];
    for api_base in refused {
        let mut case = baseline(api_base);
        case.api_key = Some(secret("caller-token"));
        let error = sign(&case, &lookup(&[]), failing()).unwrap_err();
        assert_eq!(error.to_string(), plaintext_refusal(api_base));
    }
}

fn assert_unparseable_base_is_refused() {
    let api_base = "not a url";
    let mut case = baseline(api_base);
    case.api_key = Some(secret("caller-token"));
    let error = sign(&case, &lookup(&[]), failing()).unwrap_err();
    assert_eq!(error.to_string(), plaintext_refusal(api_base));
}
