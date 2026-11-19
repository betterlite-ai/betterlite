#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    BaseClass, Constraints, JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::{
    DEFAULT_ANTHROPIC_API_VERSION, JsonMessagesPolicy, MessagesCredentials, generic_complete_url,
    generic_transform_request, generic_validate_environment,
};
use crate::api::anthropic_beta_filter::Headers;
use crate::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};

const KEY_ENV: &str = "PROBE_API_KEY";
const BASE_ENV: &str = "PROBE_API_BASE";

// the cases carry no thinking fields, so the fake never answers a capability question with data
#[derive(Default)]
struct NoCapabilities {
    // the provider each transform asks its capability questions with
    observed_provider: Option<String>,
}

impl ThinkingCapabilities for NoCapabilities {
    fn supports(
        &mut self,
        _model: &str,
        _key: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn map_effort<'seam>(
        &'seam mut self,
        _model: &'seam str,
        _effort: &'seam str,
        _provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        ready(Ok(None))
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn supports_effort_param(
        &mut self,
        _model: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        provider: &str,
    ) -> impl Future<Output = bool> + Send {
        self.observed_provider = Some(provider.to_owned());
        ready(false)
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        _request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        ready(Ok(()))
    }
}

fn descriptor(provider: ProviderId, base_url: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        provider,
        config: ProviderConfig {
            base_url: base_url.to_owned(),
            api_key_env: KEY_ENV.to_owned(),
            api_base_env: Some(BASE_ENV.to_owned()),
            base_class: BaseClass::default(),
            param_mappings: BTreeMap::new(),
            constraints: Constraints::default(),
            special_handling: BTreeMap::new(),
            supported_endpoints: Vec::new(),
        },
    }
}

const fn messages_policy(descriptor: ProviderDescriptor) -> JsonMessagesPolicy {
    JsonMessagesPolicy { descriptor }
}

fn policy(provider: ProviderId, base_url: &str) -> JsonMessagesPolicy {
    messages_policy(descriptor(provider, base_url))
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("a JSON object")
}

fn messages(source: &str) -> Vec<Json> {
    serde_json::from_str(source).expect("JSON messages")
}

fn assert_object(actual: &Object, expected: &str) {
    let expected_object = object(expected);
    assert_eq!(actual, &expected_object);
}

#[test]
fn policy_accessors_and_lookup_report_the_descriptor() {
    let policy = policy(ProviderId::Cortecs, "https://descriptor.invalid/v1");
    assert_eq!(policy.descriptor.provider, ProviderId::Cortecs);
    assert_eq!(
        policy.descriptor.config.base_url.as_str(),
        "https://descriptor.invalid/v1"
    );
    let registry = JSONProviderRegistry::load();
    let from_registry = registry
        .providers
        .get(&ProviderId::Cortecs)
        .cloned()
        .map(messages_policy)
        .expect("cortecs is pinned");
    assert_eq!(from_registry.descriptor.provider, ProviderId::Cortecs);
    assert!(registry.providers.get(&ProviderId::Openai).is_none());
}

#[rstest::rstest]
#[case::version_only("https://probe.invalid/v1", "https://probe.invalid/v1/messages")]
#[case::slash_after_v1("https://probe.invalid/v1/", "https://probe.invalid/v1/messages")]
#[case::many_slashes("https://probe.invalid///", "https://probe.invalid/v1/messages")]
#[case::complete_path(
    "https://probe.invalid/v1/messages",
    "https://probe.invalid/v1/messages"
)]
#[case::complete_path_with_slash(
    "https://probe.invalid/v1/messages/",
    "https://probe.invalid/v1/messages"
)]
#[case::other_suffix(
    "https://probe.invalid/openai",
    "https://probe.invalid/openai/v1/messages"
)]
fn generic_url_drops_one_trailing_v1_and_appends_the_path(
    #[case] api_base: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        generic_complete_url(Some(api_base)).expect("a base"),
        expected
    );
}

#[rstest::rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn generic_url_requires_a_truthy_base(#[case] api_base: Option<&str>) {
    let error = generic_complete_url(api_base).expect_err("the generic provider has no default");
    assert_eq!(
        error.to_string(),
        "api_base is required to forward Anthropic /v1/messages to a native endpoint"
    );
}

#[rstest::rstest]
#[case::explicit(
    Some("https://explicit.invalid/v1/"),
    &[(KEY_ENV, "sk-env"), (BASE_ENV, "https://env.invalid/v1")],
    "https://explicit.invalid/v1/messages"
)]
#[case::environment_declared(
    None,
    &[(KEY_ENV, "sk-env"), (BASE_ENV, "https://env.invalid/v1/")],
    "https://env.invalid/v1/messages"
)]
#[case::empty_explicit_falls_through(
    Some(""),
    &[(BASE_ENV, "https://env.invalid/v1")],
    "https://env.invalid/v1/messages"
)]
#[case::empty_environment_falls_through(None, &[(BASE_ENV, "")], "https://descriptor.invalid/v1/messages")]
#[case::descriptor_base(None, &[], "https://descriptor.invalid/v1/messages")]
fn json_url_resolves_the_base_chain(
    #[case] api_base: Option<&str>,
    #[case] environment_pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let policy = policy(ProviderId::Cortecs, "https://descriptor.invalid/v1/");
    let lookup = environment(environment_pairs);
    let credentials = MessagesCredentials {
        api_base,
        api_key: None,
        global_api_key: None,
        lookup: &lookup,
    };
    assert_eq!(policy.complete_url(&credentials).expect("a base"), expected);
}

#[test]
fn json_url_rejects_an_empty_resolved_base() {
    let policy = policy(ProviderId::Cortecs, "");
    let lookup = environment(&[]);
    let credentials = MessagesCredentials {
        api_base: None,
        api_key: None,
        global_api_key: None,
        lookup: &lookup,
    };
    assert_eq!(
        policy
            .complete_url(&credentials)
            .expect_err("an empty base is refused")
            .to_string(),
        "api_base is required to forward Anthropic /v1/messages to a native endpoint"
    );
}

#[test]
fn generic_headers_get_lowercase_defaults() {
    let key = SecretString::from("sk-explicit");
    let built = generic_validate_environment(
        headers(&[("X-Trace", "1")]),
        Some(&key),
        &Object::new(),
        &messages("[]"),
    )
    .expect("the headers build");
    assert_eq!(
        built,
        headers(&[
            ("X-Trace", "1"),
            ("authorization", "Bearer sk-explicit"),
            ("anthropic-version", DEFAULT_ANTHROPIC_API_VERSION),
            ("content-type", "application/json"),
        ])
    );
}

#[test]
fn generic_headers_never_write_an_empty_key() {
    let empty = SecretString::from("");
    let built = generic_validate_environment(
        headers(&[("X-Trace", "1")]),
        Some(&empty),
        &Object::new(),
        &messages("[]"),
    )
    .expect("the headers build");
    assert!(!built.contains_key("authorization"));
}

#[test]
fn existing_auth_headers_are_matched_case_insensitively() {
    let key = SecretString::from("sk-explicit");
    let built = generic_validate_environment(
        headers(&[
            ("X-Api-Key", "sk-given"),
            ("Anthropic-Version", "2024-01-01"),
            ("Content-Type", "application/json; charset=utf-8"),
        ]),
        Some(&key),
        &Object::new(),
        &messages("[]"),
    )
    .expect("the headers build");
    assert_eq!(
        built,
        headers(&[
            ("X-Api-Key", "sk-given"),
            ("Anthropic-Version", "2024-01-01"),
            ("Content-Type", "application/json; charset=utf-8"),
        ])
    );
}

#[test]
fn generic_headers_accept_an_existing_authorization() {
    let key = SecretString::from("sk-explicit");
    let headers = generic_validate_environment(
        headers(&[("Authorization", "Bearer sk-given")]),
        Some(&key),
        &Object::new(),
        &messages("[]"),
    )
    .expect("the headers build");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-given")
    );
    assert_eq!(headers.get("authorization"), None);
}

#[rstest::rstest]
#[case::explicit(
    Some("sk-explicit"),
    &[(KEY_ENV, "sk-env")],
    Some("sk-global"),
    "Bearer sk-explicit"
)]
#[case::empty_explicit_falls_through(Some(""), &[(KEY_ENV, "sk-env")], None, "Bearer sk-env")]
#[case::declared_environment(None, &[(KEY_ENV, "sk-env")], Some("sk-global"), "Bearer sk-env")]
#[case::global(None, &[(KEY_ENV, "")], Some("sk-global"), "Bearer sk-global")]
fn json_headers_resolve_the_key_chain(
    #[case] api_key: Option<&str>,
    #[case] environment_pairs: &[(&str, &str)],
    #[case] global: Option<&str>,
    #[case] expected: &str,
) {
    let policy = policy(ProviderId::Cortecs, "https://descriptor.invalid/v1");
    let lookup = environment(environment_pairs);
    let explicit = api_key.map(SecretString::from);
    let global_key = global.map(SecretString::from);
    let credentials = MessagesCredentials {
        api_base: Some("https://explicit.invalid/v1"),
        api_key: explicit.as_ref(),
        global_api_key: global_key.as_ref(),
        lookup: &lookup,
    };
    let headers = policy
        .validate_environment(
            Headers::new(),
            &credentials,
            &Object::new(),
            &messages("[]"),
        )
        .expect("the headers build");
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn json_headers_never_write_a_key_that_is_nowhere() {
    let policy = policy(ProviderId::Cortecs, "https://descriptor.invalid/v1");
    let lookup = environment(&[(KEY_ENV, "")]);
    let empty_global = SecretString::from("");
    let credentials = MessagesCredentials {
        api_base: None,
        api_key: None,
        global_api_key: Some(&empty_global),
        lookup: &lookup,
    };
    let headers = policy
        .validate_environment(
            Headers::new(),
            &credentials,
            &Object::new(),
            &messages("[]"),
        )
        .expect("the headers build");
    assert!(!headers.contains_key("authorization"));
}

#[test]
fn beta_spellings_collapse_and_merge_sorted() {
    let policy = policy(ProviderId::Cortecs, "https://descriptor.invalid/v1");
    let lookup = environment(&[]);
    let credentials = MessagesCredentials {
        api_base: None,
        api_key: None,
        global_api_key: None,
        lookup: &lookup,
    };
    let headers = policy
        .validate_environment(
            headers(&[("Anthropic-Beta", "probe-beta-2099-01-01")]),
            &credentials,
            &object(r#"{"speed":"fast"}"#),
            &messages("[]"),
        )
        .expect("the headers build");
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some("fast-mode-2026-02-01,probe-beta-2099-01-01")
    );
    assert_eq!(headers.get("Anthropic-Beta"), None);
}

#[test]
fn cache_control_ttl_defaults_to_false_and_reads_the_descriptor() {
    assert!(!policy(ProviderId::Cortecs, "https://x.invalid").cache_control_ttl());
    let mut enabled = descriptor(ProviderId::Cortecs, "https://x.invalid");
    enabled.config.constraints.cache_control_ttl = Some(true);
    assert!(messages_policy(enabled).cache_control_ttl());
    let mut disabled = descriptor(ProviderId::Cortecs, "https://x.invalid");
    disabled.config.constraints.cache_control_ttl = Some(false);
    assert!(!messages_policy(disabled).cache_control_ttl());
}

const REQUEST_PARAMS: &str = r#"{"max_tokens":32,"system":[{"type":"text","text":"x-anthropic-billing-header: probe"},{"type":"text","text":"kept"}]}"#;

const CACHED_MESSAGES: &str = r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral","ttl":"1h"}}]}]"#;

#[tokio::test]
async fn json_transform_strips_billing_and_the_ttl_hint() {
    let policy = policy(ProviderId::Cortecs, "https://descriptor.invalid/v1");
    let body = policy
        .transform_request(
            &mut NoCapabilities::default(),
            "example-model",
            messages(CACHED_MESSAGES),
            object(REQUEST_PARAMS),
        )
        .await
        .expect("the request builds");
    assert_object(
        &body,
        r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}],"max_tokens":32,"model":"example-model","system":[{"type":"text","text":"kept"}]}"#,
    );
}

#[tokio::test]
async fn json_transform_keeps_the_ttl_when_the_descriptor_opts_in() {
    let mut descriptor = descriptor(ProviderId::Cortecs, "https://descriptor.invalid/v1");
    descriptor.config.constraints.cache_control_ttl = Some(true);
    let body = messages_policy(descriptor)
        .transform_request(
            &mut NoCapabilities::default(),
            "example-model",
            messages(CACHED_MESSAGES),
            object(r#"{"max_tokens":32}"#),
        )
        .await
        .expect("the request builds");
    assert_object(
        &body,
        r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral","ttl":"1h"}}]}],"max_tokens":32,"model":"example-model"}"#,
    );
}

#[tokio::test]
async fn generic_transform_keeps_billing_and_strips_the_ttl_hint() {
    let body = generic_transform_request(
        &mut NoCapabilities::default(),
        "example-model",
        messages(CACHED_MESSAGES),
        object(REQUEST_PARAMS),
    )
    .await
    .expect("the request builds");
    assert_object(
        &body,
        r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}],"max_tokens":32,"model":"example-model","system":[{"type":"text","text":"x-anthropic-billing-header: probe"},{"type":"text","text":"kept"}]}"#,
    );
}

#[tokio::test]
async fn transform_without_max_tokens_is_an_error() {
    let error = generic_transform_request(
        &mut NoCapabilities::default(),
        "example-model",
        messages("[]"),
        Object::new(),
    )
    .await
    .expect_err("max_tokens is required");
    assert_eq!(
        error.to_string(),
        "max_tokens is required for Anthropic /v1/messages API"
    );
}

// the JSON subclass asks capability questions with its provider slug; the generic class stays on anthropic
#[tokio::test]
async fn the_json_subclass_reports_its_slug_and_the_generic_class_anthropic() {
    let mut json_capabilities = NoCapabilities::default();
    policy(ProviderId::Cortecs, "https://descriptor.invalid/v1")
        .transform_request(
            &mut json_capabilities,
            "example-model",
            messages("[]"),
            object(r#"{"max_tokens":32}"#),
        )
        .await
        .expect("the request builds");
    assert_eq!(
        json_capabilities.observed_provider.as_deref(),
        Some("cortecs")
    );
    let mut generic_capabilities = NoCapabilities::default();
    generic_transform_request(
        &mut generic_capabilities,
        "example-model",
        messages("[]"),
        object(r#"{"max_tokens":32}"#),
    )
    .await
    .expect("the request builds");
    assert_eq!(
        generic_capabilities.observed_provider.as_deref(),
        Some("anthropic")
    );
}
