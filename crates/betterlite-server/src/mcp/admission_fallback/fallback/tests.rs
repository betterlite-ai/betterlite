#![cfg(test)]

use std::collections::BTreeMap;
use std::io;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_mcp::oauth_resource::base_url::RequestOrigin;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;

use super::{FallbackError, FallbackFlags, admission_failure_fallback};
use crate::mcp::admission_fallback::challenge::{ChallengeContext, McpAuthHeaders};
use crate::mcp::admission_fallback::targets::TargetQuery;

const PASSTHROUGH: &str = r#"{"server_id":"srv-pt","name":"pt","transport":"http","oauth_passthrough":true,"extra_headers":["authorization"]}"#;
const PLAIN: &str = r#"{"server_id":"srv-plain","name":"plain","transport":"http","auth_type":"bearer_token","extra_headers":["authorization"]}"#;
const LEGACY_DELEGATE: &str = r#"{"server_id":"srv-legacy","name":"legacy","transport":"http","auth_type":"oauth2","delegate_auth_to_upstream":true}"#;

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn registry(records: &[&str]) -> ServerRegistry {
    let servers: Vec<MCPServer> = records.iter().copied().map(server).collect();
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        },
        Vec::new(),
        servers,
    )
}

fn query<'request>(
    registry: &'request ServerRegistry,
    route: &'request str,
    header_servers: Option<&'request [String]>,
) -> TargetQuery<'request> {
    TargetQuery {
        registry,
        route,
        header_servers,
        client_ip: None,
    }
}

fn context<'request>(base_url: &'request str, path: &'request str) -> ChallengeContext<'request> {
    ChallengeContext {
        proxy_base_url: None,
        origin: RequestOrigin {
            base_url,
            x_forwarded_proto: None,
            x_forwarded_host: None,
            x_forwarded_port: None,
            from_trusted_proxy: false,
        },
        server_root_path: "/",
        original_path: None,
        path,
        app_root_path: None,
        root_path: None,
    }
}

fn authenticate<'request>(
    mcp_auth_header: Option<&'request str>,
    server_headers: &'request BTreeMap<String, BTreeMap<String, String>>,
) -> McpAuthHeaders<'request> {
    McpAuthHeaders {
        mcp_auth_header,
        server_headers,
    }
}

fn denied() -> io::Error {
    io::Error::other("denied")
}

fn is_anonymous(auth: &UserAPIKeyAuth) -> bool {
    let empty = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
    serde_json::to_string(auth).unwrap() == serde_json::to_string(&empty).unwrap()
}

#[test]
fn passthrough_cold_start_defers_admission() {
    let registry = registry(&[PASSTHROUGH]);
    let ctx = context("http://testserver/", "/mcp/pt");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let result = admission_failure_fallback(
        &query(&registry, "/mcp/pt", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: false,
        },
        denied(),
    );
    assert!(is_anonymous(&result.unwrap()));
}

#[test]
fn passthrough_cold_start_holds_with_a_bearer() {
    let registry = registry(&[PASSTHROUGH]);
    let ctx = context("http://testserver/", "/mcp/pt");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let result = admission_failure_fallback(
        &query(&registry, "/mcp/pt", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: true,
        },
        denied(),
    );
    assert!(is_anonymous(&result.unwrap()));
}

#[test]
fn legacy_delegate_cold_start_needs_no_bearer() {
    let registry = registry(&[LEGACY_DELEGATE]);
    let ctx = context("http://testserver/", "/mcp/legacy");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let result = admission_failure_fallback(
        &query(&registry, "/mcp/legacy", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: false,
        },
        denied(),
    );
    assert!(is_anonymous(&result.unwrap()));
}

#[test]
fn legacy_delegate_with_a_bearer_keeps_the_error() {
    let registry = registry(&[LEGACY_DELEGATE]);
    let ctx = context("http://testserver/", "/mcp/legacy");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp/legacy", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: true,
        },
        denied(),
    )
    .unwrap_err();
    assert!(matches!(failure, FallbackError::Original(_)));
}

#[test]
fn client_mcp_auth_refuses_the_bypass() {
    let registry = registry(&[PASSTHROUGH]);
    let ctx = context("http://testserver/", "/mcp/pt");
    let headers = BTreeMap::new();
    let auth = authenticate(Some("Bearer upstream-token"), &headers);
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp/pt", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: false,
        },
        denied(),
    )
    .unwrap_err();
    assert!(matches!(failure, FallbackError::Original(_)));
}

#[test]
fn non_401_error_keeps_the_bypass_closed() {
    let registry = registry(&[PASSTHROUGH]);
    let ctx = context("http://testserver/", "/mcp/pt");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp/pt", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: false,
            bearer_presented: false,
        },
        denied(),
    )
    .unwrap_err();
    assert!(matches!(failure, FallbackError::Original(_)));
}

#[test]
fn header_naming_another_server_refuses_the_bypass() {
    let registry = registry(&[PASSTHROUGH, PLAIN]);
    let ctx = context("http://testserver/", "/mcp/pt");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let header_servers = vec![String::from("plain")];
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp/pt", Some(&header_servers)),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: false,
        },
        denied(),
    )
    .unwrap_err();
    assert!(matches!(failure, FallbackError::Original(_)));
}

#[test]
fn aggregate_401_returns_the_gateway_challenge() {
    let registry = registry(&[]);
    let ctx = context("http://testserver/", "/mcp");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: false,
        },
        denied(),
    )
    .unwrap_err();
    let FallbackError::Challenge(challenge) = failure else {
        panic!("the aggregate 401 must become the gateway challenge");
    };
    let url = "http://testserver/.well-known/oauth-protected-resource/mcp";
    assert_eq!(challenge.status, 401);
    assert_eq!(
        challenge.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn aggregate_401_with_a_bearer_sets_invalid_token() {
    let registry = registry(&[]);
    let ctx = context("http://testserver/", "/mcp");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: true,
        },
        denied(),
    )
    .unwrap_err();
    let FallbackError::Challenge(challenge) = failure else {
        panic!("the aggregate 401 must become the gateway challenge");
    };
    let url = "http://testserver/.well-known/oauth-protected-resource/mcp";
    assert_eq!(
        challenge.headers,
        vec![(
            "WWW-Authenticate".to_owned(),
            format!("Bearer error=\"invalid_token\", resource_metadata=\"{url}\""),
        )]
    );
}

#[test]
fn named_target_without_gateway_sign_in_keeps_the_error() {
    let registry = registry(&[PLAIN]);
    let ctx = context("http://testserver/", "/mcp/plain");
    let headers = BTreeMap::new();
    let auth = authenticate(None, &headers);
    let failure = admission_failure_fallback(
        &query(&registry, "/mcp/plain", None),
        &ctx,
        &auth,
        FallbackFlags {
            error_is_401: true,
            bearer_presented: false,
        },
        denied(),
    )
    .unwrap_err();
    let FallbackError::Original(error) = failure else {
        panic!("a target that cannot use gateway sign-in keeps its admission error");
    };
    assert_eq!(error.to_string(), "denied");
}
