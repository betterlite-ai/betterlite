#![cfg(test)]

use std::fs;
use std::path::Path;

use super::canonical::{
    RequestRouteSource, ScopeFields, get_request_route, is_assistants_api_request,
    is_generate_content_route,
};
use super::classifier::{is_azure_openai_route, is_llm_api_route, is_mcp_discovery_route};
use super::groups::get_actual_routes;
use super::method::{MethodAccess, MethodAccessError, get_request_method};
use super::templates::{
    RouteValue, check_route_access, route_matches_allowed_route, route_matches_pattern,
    route_matches_wildcard_pattern,
};
use crate::auxiliary_routes;
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
    assert!(is_azure_openai_route(&RouteValue::Text(
        "/engines/gpt-4/chat/completions\n"
    )));
    assert!(!is_azure_openai_route(&RouteValue::Text(
        "/engines/gpt-4/chat/completions\n\n"
    )));
}

#[test]
fn python_regex_matching_starts_at_zero_and_accepts_lookahead() {
    assert!(!route_matches_pattern(
        &RouteValue::Text("/prefix/bar"),
        "/foo|/bar"
    ));
    assert!(route_matches_pattern(
        &RouteValue::Text("/foo/anything"),
        "/foo|/bar"
    ));
    assert!(route_matches_pattern(
        &RouteValue::Text("/models/gpt-4"),
        "/models/(?=gpt-)[^/]+"
    ));
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
    assert!(auxiliary_routes::INFO_ROUTES.contains(&"/health"));
    assert!(!auxiliary_routes::INFO_ROUTES.contains(&"/health/"));
    assert!(check_route_access(
        "/v1/mcp/server/longer/path/with/slashes",
        &management_routes::management_routes()
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
fn google_colon_suffix_boundary() {
    let pattern = [
        "/v1beta/models/{",
        "model_name:path",
        "}",
        ":generateContent",
    ]
    .concat();
    assert!(route_matches_pattern(
        &RouteValue::Text("/v1beta/models/foo/bar:generateContent"),
        &pattern
    ));
    assert!(!route_matches_pattern(
        &RouteValue::Text("/v1beta/models/foo:bar:generateContent"),
        &pattern
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
    assert!(matches!("/foo*".chars().last(), Some('*')));
    assert!(route_matches_wildcard_pattern("/foo", "/foo*"));
    assert!(route_matches_wildcard_pattern("/foobar", "/foo*"));
}

#[test]
fn inner_star_not_wildcard() {
    assert!(!matches!("/foo*bar".chars().last(), Some('*')));
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

#[rstest::rstest]
#[case::negative_final_newline("/foo\n", r"/foo(?!\n)", false)]
#[case::positive_final_newline("/foo\n", r"/foo(?=\n)", true)]
#[case::negative_without_newline("/foo", r"/foo(?!\n)", true)]
#[case::positive_without_newline("/foo", r"/foo(?=\n)", false)]
#[case::inner_end_anchor("/foo\n", r"/foo$(?=\n)", true)]
#[case::ascii_word("/models/gpt-4", r"/models/(?a:gpt-\w+)", true)]
#[case::ascii_rejects_unicode("/models/gpt-\u{e9}", r"/models/(?a:gpt-\w+)", false)]
#[case::ascii_complement("/\u{e9}", r"/(?a:\W)", true)]
#[case::unicode_restored("/a\u{e9}", r"/(?a:\w)\w", true)]
fn python_regex_boundaries(#[case] route: &str, #[case] pattern: &str, #[case] expected: bool) {
    assert_eq!(check_route_access(route, &[pattern]), expected);
}

#[rstest::rstest]
#[case::ascii_kelvin_denied("/models/\u{212a}", "/models/(?ai:k)", false)]
#[case::unicode_multichar_denied("/models/\u{df}", "/models/(?i:ss)", false)]
#[case::unicode_flag("/models/gpt-4", r"/models/(?u:gpt-\w+)", true)]
#[case::unicode_inside_ascii("/models/\u{e9}", r"/models/(?a:(?u:\w+))", true)]
#[case::ascii_case("/models/K", "/models/(?ai:k)", true)]
#[case::unicode_kelvin("/models/\u{212a}", "/models/(?i:k)", true)]
#[case::ascii_nonascii_literal("/models/\u{c9}", "/models/(?ai:\u{e9})", false)]
#[case::ascii_after_unicode("/models/\u{e9}\u{e9}", r"/models/(?a:(?u:\w)\w)", false)]
fn python_flag_scopes(#[case] route: &str, #[case] pattern: &str, #[case] expected: bool) {
    assert_eq!(check_route_access(route, &[pattern]), expected);
}

#[rstest::rstest]
#[case::dotted_i("/\u{130}", "/(?i:i)", true)]
#[case::dotless_i("/\u{131}", "/(?i:i)", true)]
#[case::long_s("/\u{17f}", "/(?i:s)", true)]
#[case::range_dotted_i("/\u{130}", "/(?i:[a-z])", true)]
#[case::range_dotless_i("/\u{131}", "/(?i:[A-Z])", true)]
#[case::negated_range("/\u{130}", "/(?i:[^a-z])", false)]
#[case::ascii_range_kelvin("/\u{212a}", "/(?ai:[a-z])", false)]
#[case::ascii_range_accent("/\u{c9}", "/(?ai:[\u{e9}])", false)]
#[case::unicode_range_accent("/\u{c9}", "/(?i:[\u{e9}])", true)]
#[case::ascii_complement("/\u{e9}", r"/(?a:[\W])", true)]
#[case::unicode_number("/\u{b2}", r"/\w", true)]
#[case::unicode_mark("/\u{301}", r"/\w", false)]
#[case::unicode_space("/\u{1c}", r"/\s", true)]
#[case::ascii_space("/\u{1c}", r"/(?a:\s)", false)]
#[case::nested_ignorecase("/aK\u{e9}", "/(?i:a(?a:k)(?-i:\u{e9}))", true)]
#[case::restored_ignorecase("/aK\u{c9}", "/(?i:a(?a:k)(?-i:\u{e9}))", false)]
#[case::escaped_literal("/\u{212a}", r"/(?i:\x6b)", true)]
#[case::escaped_ascii_literal("/\u{212a}", r"/(?ai:\x6b)", false)]
#[case::lookbehind("/K\u{e9}", "/(?ai:k)(?<=K)(?u:\u{e9})", true)]
#[case::scoped_negative_lookahead("/\u{212a}", "/(?!(?ai:k))(?i:k)", true)]
#[case::dot_newline("/a\nb", "/(?s:a.b)", true)]
#[case::verbose_comment("/K", "/(?ix: # ) ignored\n k)", true)]
#[case::class_brackets("/]", "/(?i:[]a])", true)]
#[case::class_literal_open("/[", "/(?i:[[])", true)]
#[case::unicode_boundary("/\u{b2}", r"/\b\w+\b", true)]
#[case::ascii_boundary("/\u{e9}", r"/(?a:\b)\u00e9", false)]
#[case::strict_end("/foo\n", r"/foo\Z", false)]
#[case::named_capture("/aa", "/(?P<letter>a)(?P=letter)", true)]
#[case::capture_conditional("/ab", "/(a)?(?(1)b|c)", true)]
fn python_regex_adapter(#[case] route: &str, #[case] pattern: &str, #[case] expected: bool) {
    assert_eq!(check_route_access(route, &[pattern]), expected);
}

#[test]
fn replays_the_route_pattern_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../betterlite-server/fuzz/corpus/route_pattern");
    let results = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let source = fs::read_to_string(entry.unwrap().path()).unwrap();
            let (pattern, route) = source.split_once('\0').unwrap();
            route_matches_pattern(&RouteValue::Text(route), pattern)
        })
        .collect::<Vec<_>>();
    assert!(!results.is_empty());
}

#[test]
fn python_regex_resource_limits() {
    assert!(matches!(
        super::python_regex::compile(&"a".repeat(0x4001)),
        Err(super::python_regex::RegexError::Limit)
    ));
    let deeply_nested = format!("{}a{}", "(?:".repeat(129), ")".repeat(129));
    assert!(matches!(
        super::python_regex::compile(&deeply_nested),
        Err(super::python_regex::RegexError::Limit)
    ));
    let expensive = format!("{}!", "a".repeat(100));
    assert!(!route_matches_pattern(
        &RouteValue::Text(&expensive),
        "(a+)+"
    ));
}

#[test]
fn invalid_route_regex_denies() {
    assert!(!route_matches_pattern(
        &RouteValue::Text("/value"),
        "/(?a:["
    ));
}
