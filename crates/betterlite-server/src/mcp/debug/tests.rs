#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use rstest::rstest;
use secrecy::SecretString;

use super::auth_diagnostics::MCPAuthDiagnostics;
use super::headers::{DebugHeaderInputs, DebugServer, build_debug_headers};
use super::{DebugSendHeaders, is_debug_enabled, mask_secret, maybe_build_debug_headers};

fn raw(pairs: &[(&str, &str)]) -> RawHeaders {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    RawHeaders::from_pairs(&bytes).expect("distinct headers")
}

fn secret(value: &str) -> SecretString {
    SecretString::from(String::from(value))
}

#[rstest]
#[case::missing(None, "(none)")]
#[case::empty(Some(""), "(none)")]
#[case::short(Some("abc"), "***")]
#[case::at_the_visible_limit(Some("abcdefghij"), "**********")]
#[case::one_past_the_limit(Some("abcdefghijk"), "abcdef*hijk")]
#[case::bearer_token(Some("sk-1234567890abcdef"), "sk-123*********cdef")]
fn masking_keeps_six_and_four(#[case] value: Option<&str>, #[case] expected: &str) {
    assert_eq!(mask_secret(value), expected);
}

#[rstest]
#[case::absent(&[], false)]
#[case::lowercase(&[("x-litellm-mcp-debug", "true")], true)]
#[case::mixed_case_name(&[("X-LiteLLM-MCP-Debug", "TRUE")], true)]
#[case::padded_one(&[("x-litellm-mcp-debug", " 1 ")], true)]
#[case::yes(&[("x-litellm-mcp-debug", "yes")], true)]
#[case::on_is_not_accepted(&[("x-litellm-mcp-debug", "on")], false)]
#[case::empty_value(&[("x-litellm-mcp-debug", "")], false)]
#[case::suffixed_name(&[("x-litellm-mcp-debug-extra", "true")], false)]
fn reading_the_debug_header(#[case] pairs: &[(&str, &str)], #[case] expected: bool) {
    assert_eq!(is_debug_enabled(&raw(pairs)), expected);
}

#[test]
fn the_first_matching_debug_header_decides() {
    let headers = raw(&[
        ("x-litellm-mcp-debug", "false"),
        ("x-litellm-api-key", "sk-1234567890abcdef"),
    ]);
    assert!(!is_debug_enabled(&headers));
}

#[test]
fn headers_without_any_value_show_their_sentinels() {
    let headers = raw(&[]);
    let built = build_debug_headers(&DebugHeaderInputs {
        inbound_headers: &headers,
        oauth2_headers: None,
        litellm_api_key: None,
        auth_resolution: AuthResolution::Unresolved,
        server_url: None,
        server_auth_type: None,
    });
    assert_eq!(
        built,
        vec![
            (
                String::from("x-mcp-debug-inbound-auth"),
                String::from("(none)")
            ),
            (
                String::from("x-mcp-debug-oauth2-token"),
                String::from("(none)")
            ),
            (
                String::from("x-mcp-debug-auth-resolution"),
                String::from("unresolved")
            ),
            (
                String::from("x-mcp-debug-outbound-url"),
                String::from("(unknown)")
            ),
            (
                String::from("x-mcp-debug-server-auth-type"),
                String::from("(none)")
            ),
        ]
    );
}

#[test]
fn inbound_auth_headers_are_masked_and_summarised() {
    let headers = raw(&[
        ("X-LiteLLM-API-Key", "Bearer sk-1234567890abcdef"),
        ("authorization", "Bearer oauth-token-1234567890"),
        ("x-mcp-auth", "abc"),
    ]);
    let built = build_debug_headers(&DebugHeaderInputs {
        inbound_headers: &headers,
        oauth2_headers: None,
        litellm_api_key: None,
        auth_resolution: AuthResolution::NoAuth,
        server_url: None,
        server_auth_type: None,
    });
    assert_eq!(
        built.first().expect("inbound auth").1,
        "x-litellm-api-key=Bearer****************cdef; authorization=Bearer*******************7890; x-mcp-auth=***"
    );
    assert_eq!(built.get(2).expect("resolution").1, "no-auth");
}

#[test]
fn an_oauth_token_matching_the_litellm_key_is_flagged() {
    let headers = raw(&[("authorization", "Bearer sk-1234567890abcdef")]);
    let oauth2 = BTreeMap::from([(
        String::from("Authorization"),
        secret("Bearer sk-1234567890abcdef"),
    )]);
    let built = build_debug_headers(&DebugHeaderInputs {
        inbound_headers: &headers,
        oauth2_headers: Some(&oauth2),
        litellm_api_key: Some(&secret("Bearer sk-1234567890abcdef")),
        auth_resolution: AuthResolution::Oauth2Passthrough,
        server_url: Some("https://mcp.example.com/mcp"),
        server_auth_type: Some("oauth2"),
    });
    assert_eq!(
        built.get(1).expect("token").1,
        "Bearer****************cdef (SAME_AS_LITELLM_KEY - likely misconfigured)"
    );
}

#[test]
fn a_different_oauth_token_is_only_masked() {
    let headers = raw(&[]);
    let oauth2 = BTreeMap::from([(
        String::from("Authorization"),
        secret("Bearer oauth-token-1234567890"),
    )]);
    let built = build_debug_headers(&DebugHeaderInputs {
        inbound_headers: &headers,
        oauth2_headers: Some(&oauth2),
        litellm_api_key: Some(&secret("sk-1234567890abcdef")),
        auth_resolution: AuthResolution::StaticToken,
        server_url: Some("https://mcp.example.com/mcp"),
        server_auth_type: Some("api_key"),
    });
    assert_eq!(
        built.get(1).expect("token").1,
        "Bearer*******************7890"
    );
    assert_eq!(built.get(3).expect("url").1, "https://mcp.example.com/mcp");
    assert_eq!(built.get(4).expect("auth type").1, "api_key");
}

#[test]
fn empty_server_values_take_their_sentinels() {
    let headers = raw(&[]);
    let built = build_debug_headers(&DebugHeaderInputs {
        inbound_headers: &headers,
        oauth2_headers: None,
        litellm_api_key: Some(&secret("")),
        auth_resolution: AuthResolution::Unresolved,
        server_url: Some(""),
        server_auth_type: Some(""),
    });
    assert_eq!(built.get(3).expect("url").1, "(unknown)");
    assert_eq!(built.get(4).expect("auth type").1, "(none)");
}

#[test]
fn maybe_builds_nothing_without_the_debug_header() {
    let headers = raw(&[("x-litellm-api-key", "sk-1234567890abcdef")]);
    let servers = [DebugServer {
        url: Some("https://alpha.test/mcp"),
        auth_type: Some("oauth2"),
    }];
    assert!(maybe_build_debug_headers(Some(&headers), &servers, None).is_none());
    assert!(maybe_build_debug_headers(None, &servers, None).is_none());
}

#[test]
fn maybe_builds_the_first_server_and_the_header_key() {
    let headers = raw(&[
        ("x-litellm-mcp-debug", "true"),
        ("x-litellm-api-key", "Bearer sk-1234567890abcdef"),
    ]);
    let servers = [DebugServer {
        url: Some("https://alpha.test/mcp"),
        auth_type: Some("oauth2"),
    }];
    let built = maybe_build_debug_headers(Some(&headers), &servers, None).expect("debug enabled");
    assert_eq!(built.get(2).expect("resolution").1, "unresolved");
    assert_eq!(built.get(3).expect("url").1, "https://alpha.test/mcp");
    assert_eq!(built.get(4).expect("auth type").1, "oauth2");
    assert_eq!(
        built.first().expect("inbound auth").1,
        "x-litellm-api-key=Bearer****************cdef"
    );
}

#[test]
fn maybe_builds_without_any_resolved_server() {
    let headers = raw(&[("x-litellm-mcp-debug", "true")]);
    let built = maybe_build_debug_headers(Some(&headers), &[], None).expect("debug enabled");
    assert_eq!(built.get(3).expect("url").1, "(unknown)");
    assert_eq!(built.get(4).expect("auth type").1, "(none)");
}

#[test]
fn the_post_wrapper_overrides_the_resolution_header_in_place() {
    let mut diagnostics = MCPAuthDiagnostics::default();
    diagnostics.record("s1", AuthResolution::StaticToken);
    let send = DebugSendHeaders {
        debug_headers: vec![
            (
                String::from("x-mcp-debug-outbound-url"),
                String::from("https://alpha.test/mcp"),
            ),
            (
                String::from("x-mcp-debug-auth-resolution"),
                String::from("unresolved"),
            ),
        ],
        diagnostics: Some(&diagnostics),
    };
    assert_eq!(
        send.headers(),
        vec![
            (
                String::from("x-mcp-debug-outbound-url"),
                String::from("https://alpha.test/mcp"),
            ),
            (
                String::from("x-mcp-debug-auth-resolution"),
                String::from("static-token"),
            ),
        ]
    );
}

#[test]
fn the_wrapper_appends_new_resolution_headers() {
    let mut diagnostics = MCPAuthDiagnostics::default();
    diagnostics.record("s1", AuthResolution::StaticToken);
    diagnostics.record("s2", AuthResolution::NoAuth);
    let send = DebugSendHeaders {
        debug_headers: vec![(
            String::from("x-mcp-debug-outbound-url"),
            String::from("https://alpha.test/mcp"),
        )],
        diagnostics: Some(&diagnostics),
    };
    let headers = send.headers();
    assert_eq!(headers.len(), 3);
    assert_eq!(headers.get(1).expect("resolution").1, "multiple");
    assert_eq!(
        headers.get(2).expect("resolutions").1,
        "{\"s1\":\"static-token\",\"s2\":\"no-auth\"}"
    );
}

#[test]
fn a_non_post_wrapper_sends_the_built_headers() {
    let diagnostics = MCPAuthDiagnostics::default();
    let send = DebugSendHeaders {
        debug_headers: vec![(
            String::from("x-mcp-debug-auth-resolution"),
            String::from("unresolved"),
        )],
        diagnostics: None,
    };
    assert_eq!(
        send.headers(),
        vec![(
            String::from("x-mcp-debug-auth-resolution"),
            String::from("unresolved")
        )]
    );
    assert_eq!(diagnostics.headers().len(), 1);
}
