#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;

use super::{
    DEFAULT_EXCLUDED_ROUTES, EXCLUDED_URLS_ENV, PASSTHROUGH_PREFIXES, PassthroughSpanName,
    ServerSpanPolicy, ServerSpanPolicyError, passthrough_span_name,
};

struct TestEnv {
    values: BTreeMap<String, String>,
    lookups: Mutex<Vec<String>>,
}

impl EnvironmentLookup for TestEnv {
    fn lookup(&self, name: &str) -> Option<String> {
        self.lookups.lock().expect("lookups").push(name.to_owned());
        self.values.get(name).cloned()
    }
}

fn env(pairs: &[(&str, &str)]) -> TestEnv {
    TestEnv {
        values: pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        lookups: Mutex::new(Vec::new()),
    }
}

fn policy(environment: &TestEnv) -> ServerSpanPolicy {
    ServerSpanPolicy::from_gate(true, environment)
        .expect("policy")
        .expect("gate on")
}

#[test]
fn the_default_list_is_the_eleven_routes_in_order() {
    assert_eq!(
        DEFAULT_EXCLUDED_ROUTES,
        [
            "/health",
            "/metrics",
            "/litellm-asset-prefix",
            "/_next",
            "/ui",
            "/swagger",
            "/docs",
            "/redoc",
            "/openapi.json",
            "favicon",
            "/.well-known",
        ]
    );
}

#[test]
fn the_excluded_urls_variable_is_the_standard_instrumentation_name() {
    assert_eq!(EXCLUDED_URLS_ENV, "OTEL_PYTHON_FASTAPI_EXCLUDED_URLS");
}

#[rstest]
#[case("/health", true)]
#[case("/health/readiness", true)]
#[case("/healthz", true)]
#[case("/metrics", true)]
#[case("/litellm-asset-prefix/logo.svg", true)]
#[case("/_next/static/app.js", true)]
#[case("/ui", true)]
#[case("/user/ui-config", true)]
#[case("/swagger/index.html", true)]
#[case("/docs", true)]
#[case("/docs-private", true)]
#[case("/redoc", true)]
#[case("/openapi.json", true)]
#[case("/v1/favicon.ico", true)]
#[case("/.well-known/openid-configuration", true)]
#[case("/v1/chat/completions", false)]
#[case("/key/generate", false)]
#[case("/model/info", false)]
fn the_default_list_is_matched_as_an_unanchored_search(#[case] path: &str, #[case] excluded: bool) {
    let environment = env(&[]);
    assert_eq!(policy(&environment).is_excluded(path), excluded);
}

#[test]
fn the_override_replaces_the_whole_default_list() {
    let environment = env(&[(EXCLUDED_URLS_ENV, "^/custom$")]);
    let policy = policy(&environment);
    assert!(policy.is_excluded("/custom"));
    assert!(!policy.is_excluded("/health/readiness"));
}

#[test]
fn an_empty_override_excludes_nothing() {
    let environment = env(&[(EXCLUDED_URLS_ENV, "")]);
    let policy = policy(&environment);
    assert!(!policy.is_excluded("/health"));
    assert!(!policy.is_excluded("/metrics"));
}

#[test]
fn the_override_splits_on_a_comma_and_the_spaces_after_it() {
    let environment = env(&[(EXCLUDED_URLS_ENV, "^/a$,  ^/b$ ,^/c$")]);
    let policy = policy(&environment);
    assert!(policy.is_excluded("/a"));
    assert!(policy.is_excluded("/b"));
    assert!(policy.is_excluded("/c"));
    assert!(!policy.is_excluded("/health"));
}

#[test]
fn an_entry_that_is_not_a_regex_is_an_error() {
    let environment = env(&[(EXCLUDED_URLS_ENV, "^/ok$, [unclosed")]);
    let result = ServerSpanPolicy::from_gate(true, &environment);
    let Err(error) = result else {
        panic!("an invalid entry must be an error");
    };
    assert!(matches!(
        error,
        ServerSpanPolicyError::InvalidExcludedUrl { .. }
    ));
    let text = error.to_string();
    assert!(text.contains(EXCLUDED_URLS_ENV));
    assert!(text.contains("[unclosed"));
}

#[test]
fn the_gate_off_builds_no_policy_and_reads_no_environment() {
    let environment = env(&[(EXCLUDED_URLS_ENV, "[unclosed")]);
    assert!(
        ServerSpanPolicy::from_gate(false, &environment)
            .expect("gate off")
            .is_none()
    );
    assert!(environment.lookups.lock().expect("lookups").is_empty());
}

#[test]
fn the_passthrough_prefixes_are_sixteen_distinct_entries() {
    let prefixes: BTreeSet<&str> = PASSTHROUGH_PREFIXES.into_iter().collect();
    assert_eq!(prefixes.len(), 16);
}

#[rstest]
#[case("openai")]
#[case("openai_passthrough")]
#[case("anthropic")]
#[case("azure")]
#[case("azure_ai")]
#[case("bedrock")]
#[case("cohere")]
#[case("cursor")]
#[case("gemini")]
#[case("mistral")]
#[case("vllm")]
#[case("vertex_ai")]
#[case("vertex-ai")]
#[case("assemblyai")]
#[case("eu.assemblyai")]
#[case("milvus")]
fn every_passthrough_prefix_renames_the_span_to_the_request_path(#[case] prefix: &str) {
    let path = format!("/{prefix}/v1/chat/completions");
    assert_eq!(
        passthrough_span_name("POST", &path),
        Some(PassthroughSpanName {
            name: format!("POST {path}"),
            route: path,
        })
    );
}

#[rstest]
#[case("/v1/chat/completions")]
#[case("/health")]
#[case("/ui/playground")]
#[case("/openaiish/v1/chat")]
#[case("/v1/openai/chat")]
#[case("")]
#[case("/")]
fn a_path_outside_a_prefix_keeps_its_route_template(#[case] path: &str) {
    assert_eq!(passthrough_span_name("POST", path), None);
}

#[test]
fn leading_slashes_are_ignored_but_the_route_keeps_them() {
    assert_eq!(
        passthrough_span_name("GET", "///openai/v1/models"),
        Some(PassthroughSpanName {
            name: "GET ///openai/v1/models".to_owned(),
            route: "///openai/v1/models".to_owned(),
        })
    );
}

#[test]
fn an_empty_method_leaves_only_the_path_as_the_name() {
    assert_eq!(
        passthrough_span_name("", "/openai/v1/models"),
        Some(PassthroughSpanName {
            name: "/openai/v1/models".to_owned(),
            route: "/openai/v1/models".to_owned(),
        })
    );
}
