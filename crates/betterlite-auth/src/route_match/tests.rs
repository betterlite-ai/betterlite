#![cfg(test)]

use super::canonical::{
    RequestRouteSource, ScopeFields, get_request_route, is_assistants_api_request,
    is_generate_content_route,
};
use super::classifier::{
    is_azure_openai_route, is_info_route, is_llm_api_route, is_management_route,
    is_mcp_discovery_route,
};
use super::groups::get_actual_routes;
use super::method::{MethodAccess, MethodAccessError, get_request_method};
use super::templates::{
    RouteValue, check_route_access, is_wildcard_pattern, route_matches_allowed_route,
    route_matches_pattern, route_matches_wildcard_pattern,
};
use crate::inference_routes::ANTHROPIC_ROUTES;
use crate::management_routes;

#[test]
fn azure_alias() {
    let path = RouteValue::Text("/openai/deployments/gpt-4/chat/completions");
    assert!(is_llm_api_route(&path));
    let engine = RouteValue::Text("/engines/gpt-4/chat/completions");
    assert!(is_azure_openai_route(&engine));
}

#[test]
fn bedrock_invoke() {
    assert!(is_llm_api_route(&RouteValue::Text("/bedrock/invoke")));
    assert!(!is_llm_api_route(&RouteValue::Text("/bedrock-impostor")));
}

#[test]
fn path_template() {
    let pattern = "/v1/batches/{batch_id}/cancel";
    assert!(route_matches_pattern(
        &RouteValue::Text("/v1/batches/123/cancel"),
        pattern
    ));
    assert!(!route_matches_pattern(
        &RouteValue::Text("/v1/batches//cancel"),
        pattern
    ));
    assert!(!route_matches_pattern(
        &RouteValue::Text("/v1/batches/123/456/cancel"),
        pattern
    ));
}

#[test]
fn wildcard_boundary() {
    assert!(route_matches_wildcard_pattern(
        "/containers/",
        "/containers/*"
    ));
    assert!(route_matches_wildcard_pattern(
        "/containers/abc",
        "/containers/*"
    ));
    assert!(!route_matches_wildcard_pattern(
        "/containers",
        "/containers/*"
    ));
}

#[test]
fn allowed_route_enum_expansion() {
    assert_eq!(get_actual_routes(&["anthropic_routes"]), ANTHROPIC_ROUTES);
    let expanded = get_actual_routes(&["management_routes"]);
    assert_eq!(expanded, management_routes::management_routes());
}

#[test]
fn sibling_prefix_denied() {
    assert!(!route_matches_allowed_route(
        "/fake-openai-proxy-600",
        "/fake-openai-proxy"
    ));
    assert!(route_matches_allowed_route(
        "/fake-openai-proxy/completions",
        "/fake-openai-proxy"
    ));
}

#[test]
fn classifier_table_groups() {
    assert!(is_llm_api_route(&RouteValue::Text("/v1/chat/completions")));
    assert!(is_llm_api_route(&RouteValue::Text("/v1/messages")));
    assert!(is_llm_api_route(&RouteValue::Text("/interactions")));
    assert!(is_llm_api_route(&RouteValue::Text("/mcp")));
    assert!(is_llm_api_route(&RouteValue::Text("/agents")));
    assert!(is_llm_api_route(&RouteValue::Text("/rag/ingest")));
}

#[test]
fn nonstring_route_false() {
    assert!(!is_llm_api_route(&RouteValue::NonString));
    assert!(!is_azure_openai_route(&RouteValue::NonString));
    assert!(!route_matches_pattern(
        &RouteValue::NonString,
        "/v1/chat/completions"
    ));
}

#[test]
fn azure_two_segments_and_engine() {
    assert!(is_azure_openai_route(&RouteValue::Text(
        "/openai/deployments/gpt-4/2023-05-15/chat/completions"
    )));
    assert!(!is_azure_openai_route(&RouteValue::Text(
        "/openai/deployments/gpt-4/chat/completions"
    )));
    assert!(is_azure_openai_route(&RouteValue::Text(
        "/engines/gpt-4/chat/completions"
    )));
}

#[test]
fn mcp_get_one_segment_only() {
    assert!(is_mcp_discovery_route("/v1/mcp/server", Some("GET")));
    assert!(is_mcp_discovery_route("/v1/mcp/server/abc", Some("GET")));
    assert!(!is_mcp_discovery_route(
        "/v1/mcp/server/abc/def",
        Some("GET")
    ));
    assert!(!is_mcp_discovery_route("/v1/mcp/server/", Some("GET")));
    assert!(!is_mcp_discovery_route("/v1/mcp/server/abc", Some("PUT")));
    assert!(!is_mcp_discovery_route("/v1/mcp/server/abc", None));
}

#[test]
fn info_exact_management_patterns() {
    assert!(is_info_route("/health"));
    assert!(!is_info_route("/health/"));
    assert!(is_management_route(
        "/v1/mcp/server/longer/path/with/slashes"
    ));
}

#[test]
fn placeholder_nonempty_segment() {
    let pattern = "/v1/batches/{batch_id}/cancel";
    assert!(route_matches_pattern(
        &RouteValue::Text("/v1/batches/123/cancel"),
        pattern
    ));
    assert!(!route_matches_pattern(
        &RouteValue::Text("/v1/batches//cancel"),
        pattern
    ));
}

#[test]
fn path_placeholder_slash_colon_newline() {
    let pattern = ["/v1/mcp/server/", "{", "path:path", "}"].concat();
    assert!(route_matches_pattern(
        &RouteValue::Text("/v1/mcp/server/a/b:c\nmore"),
        &pattern
    ));
}

#[test]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the {model_name:path} here is a LiteLLM route placeholder, not a format argument"
)]
fn google_colon_suffix_boundary() {
    let pattern = "/v1beta/models/{model_name:path}:generateContent";
    assert!(route_matches_pattern(
        &RouteValue::Text("/v1beta/models/foo/bar:generateContent"),
        pattern
    ));
    assert!(!route_matches_pattern(
        &RouteValue::Text("/v1beta/models/foo:bar:generateContent"),
        pattern
    ));
}

#[test]
fn regex_metacharacters_and_final_newline() {
    assert!(route_matches_pattern(
        &RouteValue::Text("/v1/chat/completions\n"),
        "/v1/chat/completions"
    ));
    assert!(!route_matches_pattern(
        &RouteValue::Text("/v1/chat/completions\n\n"),
        "/v1/chat/completions"
    ));
    let pattern = "/a2a/{agent_id}/.well-known/agent-card.json";
    assert!(route_matches_pattern(
        &RouteValue::Text("/a2a/123/.well-known/agent-card.json"),
        pattern
    ));
    assert!(route_matches_pattern(
        &RouteValue::Text("/a2a/123/Xwell-known/agent-card.json"),
        pattern
    ));
}

#[test]
fn final_star_empty_suffix() {
    assert!(is_wildcard_pattern("/foo*"));
    assert!(route_matches_wildcard_pattern("/foo", "/foo*"));
    assert!(route_matches_wildcard_pattern("/foobar", "/foo*"));
}

#[test]
fn inner_star_not_wildcard() {
    assert!(!is_wildcard_pattern("/foo*bar"));
    assert!(check_route_access("/fobar", &["/foo*bar"]));
}

#[test]
fn empty_allowlist_denies() {
    assert!(!check_route_access("/v1/chat/completions", &[]));
}

#[test]
fn expansion_unknown_and_duplicates() {
    let expanded = get_actual_routes(&["not_a_real_group", "not_a_real_group", "anthropic_routes"]);
    let mut expected = vec!["not_a_real_group".to_owned(), "not_a_real_group".to_owned()];
    expected.extend(ANTHROPIC_ROUTES.iter().map(|route| (*route).to_owned()));
    assert_eq!(expanded, expected);
}

#[test]
fn method_missing_nonstrings() {
    assert_eq!(get_request_method(MethodAccess::NoRequest), Ok(None));
    assert_eq!(get_request_method(MethodAccess::Unavailable), Ok(None));
    assert_eq!(get_request_method(MethodAccess::NonString), Ok(None));
}

#[test]
fn method_other_error_propagates() {
    assert_eq!(
        get_request_method(MethodAccess::Other("boom".to_owned())),
        Err(MethodAccessError::Other("boom".to_owned()))
    );
    assert_eq!(
        get_request_method(MethodAccess::Text("get")),
        Ok(Some("GET".to_owned()))
    );
}

#[test]
fn assistant_substrings() {
    let threads = RequestRouteSource {
        scope: None,
        url_path: "/v1/threads/abc",
    };
    assert!(is_assistants_api_request(&threads));
    let uppercase = RequestRouteSource {
        scope: None,
        url_path: "/v1/THREADS/abc",
    };
    assert!(!is_assistants_api_request(&uppercase));
    let chat = RequestRouteSource {
        scope: None,
        url_path: "/v1/chat/completions",
    };
    assert!(!is_assistants_api_request(&chat));
}

#[test]
fn generate_content_case() {
    assert!(is_generate_content_route(
        "/v1beta/models/gemini:generateContent"
    ));
    assert!(is_generate_content_route(
        "/v1beta/models/gemini:streamGenerateContent"
    ));
    assert!(!is_generate_content_route(
        "/v1beta/models/gemini:generatecontent"
    ));
}

#[test]
fn scope_path_beats_host_url() {
    let source = RequestRouteSource {
        scope: Some(ScopeFields {
            path: Some("/scope/path"),
            app_root_path: None,
            root_path: None,
        }),
        url_path: "/host/derived/path",
    };
    assert_eq!(get_request_route(&source), "/scope/path");
}

#[test]
fn app_root_precedence() {
    let source = RequestRouteSource {
        scope: Some(ScopeFields {
            path: Some("/app/sub/resource"),
            app_root_path: Some("/app"),
            root_path: Some("/other"),
        }),
        url_path: "/unused",
    };
    assert_eq!(get_request_route(&source), "/sub/resource");
}

#[test]
fn root_exact_and_sibling() {
    let exact = RequestRouteSource {
        scope: Some(ScopeFields {
            path: Some("/app"),
            app_root_path: Some("/app"),
            root_path: None,
        }),
        url_path: "/unused",
    };
    assert_eq!(get_request_route(&exact), "/");

    let sibling = RequestRouteSource {
        scope: Some(ScopeFields {
            path: Some("/app2/resource"),
            app_root_path: Some("/app"),
            root_path: None,
        }),
        url_path: "/unused",
    };
    assert_eq!(get_request_route(&sibling), "/app2/resource");
}

#[test]
fn route_getter_debug_fallback() {
    let source = RequestRouteSource {
        scope: None,
        url_path: "/raw/fallback/path",
    };
    assert_eq!(get_request_route(&source), "/raw/fallback/path");
}
