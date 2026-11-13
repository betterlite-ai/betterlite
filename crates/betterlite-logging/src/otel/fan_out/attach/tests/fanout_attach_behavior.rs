#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rstest::rstest;

use crate::otel::config::destination::SpanScope;
use crate::otel::config::{ExporterOwner, ExporterSpec};
use crate::otel::fan_out::attach::{
    TenantFanOutRegistry, attach_tenant_fan_out, credential_name, operator_sink_scopes, sink_key,
};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::fan_out::tests::support::{FixedScope, factories, langfuse_spec, settings, spec};

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::lowercased("authorization", "authorization")]
#[case::hyphenated("Arize-Space-Id", "space_id")]
#[case::underscored("ARIZE_SPACE_ID", "space_id")]
#[case::spaced(" x-api-key ", "x_api_key")]
#[case::not_an_alias("space_id", "space_id")]
fn credential_names_fold_casing_and_aliases(#[case] header: &str, #[case] expected: &str) {
    assert_eq!(credential_name(header), expected);
}

#[rstest]
#[case::base_endpoint(Some("http://collector:4318"), "http://collector:4318/v1/traces")]
#[case::traces_path(
    Some("http://collector:4318/v1/traces"),
    "http://collector:4318/v1/traces"
)]
#[case::sibling_signal(
    Some("http://collector:4318/v1/logs"),
    "http://collector:4318/v1/traces"
)]
#[case::splunk_path(
    Some("https://splunk.example/v2/trace/otlp"),
    "https://splunk.example/v2/trace/otlp"
)]
#[case::trailing_slash(Some("http://collector:4318/"), "http://collector:4318/v1/traces")]
fn a_sink_endpoint_names_the_traces_signal(#[case] endpoint: Option<&str>, #[case] expected: &str) {
    let key = sink_key(endpoint, &headers(&[])).expect("a key");
    assert_eq!(key.0, expected);
}

#[test]
fn a_sink_without_an_endpoint_has_no_identity() {
    assert!(sink_key(None, &headers(&[])).is_none());
}

#[test]
fn an_empty_endpoint_is_still_an_account() {
    let key = sink_key(Some(""), &headers(&[])).expect("a key");
    assert_eq!(key.0, "");
}

#[test]
fn a_sink_key_sorts_credentials_by_their_normalized_name() {
    let key = sink_key(
        Some("http://collector:4318"),
        &headers(&[
            ("Zebra-Key", "z"),
            ("arize-space-id", "space"),
            ("authorization", "bearer"),
        ]),
    )
    .expect("a key");
    assert_eq!(
        key.1,
        vec![
            ("authorization".to_owned(), "bearer".to_owned()),
            ("space_id".to_owned(), "space".to_owned()),
            ("zebra_key".to_owned(), "z".to_owned()),
        ]
    );
}

#[test]
fn one_account_spelled_two_ways_has_one_scope() {
    let arize = spec(
        "otlp_http",
        Some("http://collector:4318"),
        Some("arize-space-id=space"),
    );
    let mut langfuse = spec(
        "otlp_http",
        Some("http://collector:4318/v1/traces"),
        Some("ARIZE_SPACE_ID=space"),
    );
    langfuse.owner = Some(ExporterOwner::LangfuseOtel);
    let scopes = operator_sink_scopes(
        &[
            settings(vec![arize], SpanScope::Full),
            settings(vec![langfuse], SpanScope::LlmOnly),
        ],
        &factories(),
    );
    assert_eq!(scopes.len(), 1);
    assert_eq!(
        scopes.values().next().copied(),
        Some(SpanScope::Full),
        "the wider scope wins for the account"
    );
}

#[test]
fn an_llm_only_langfuse_config_narrows_its_own_account() {
    let scopes = operator_sink_scopes(
        &[settings(
            vec![langfuse_spec("http://langfuse:4318")],
            SpanScope::LlmOnly,
        )],
        &factories(),
    );
    assert_eq!(scopes.values().next().copied(), Some(SpanScope::LlmOnly));
}

#[rstest]
#[case::no_endpoint(spec("otlp_http", None, None), false)]
#[case::console(spec("console", Some("http://collector:4318"), None), false)]
#[case::unregistered_kind(spec("jaeger", Some("http://collector:4318"), None), false)]
#[case::otlp_http(spec("otlp_http", Some("http://collector:4318"), None), true)]
#[case::with_headers(
    spec("otlp_http", Some("http://collector:4318"), Some("x-api-key=k")),
    true
)]
fn only_exporters_that_reach_the_wire_get_a_scope(
    #[case] exporter: ExporterSpec,
    #[case] scoped: bool,
) {
    let scopes = operator_sink_scopes(&[settings(vec![exporter], SpanScope::Full)], &factories());
    assert_eq!(!scopes.is_empty(), scoped);
}

#[test]
fn a_headers_required_exporter_without_headers_is_left_out() {
    let mut gated = spec("otlp_http", Some("http://newrelic:4318"), None);
    gated.requires_headers = true;
    let mut provided = spec("otlp_http", Some("http://provided:4318"), Some(""));
    provided.requires_headers = true;
    let scopes = operator_sink_scopes(
        &[settings(vec![gated, provided], SpanScope::Full)],
        &factories(),
    );
    let endpoints: Vec<&str> = scopes.keys().map(|key| key.0.as_str()).collect();
    assert_eq!(endpoints, vec!["http://provided:4318/v1/traces"]);
}

#[test]
fn attaching_twice_leaves_one_fan_out() {
    let mut registry = TenantFanOutRegistry::default();
    let scope = FixedScope::new(vec![], false);
    let first_config = settings(
        vec![spec("otlp_http", Some("http://first:4318"), None)],
        SpanScope::Full,
    );
    let second_config = settings(vec![], SpanScope::Full);
    let concrete = Arc::clone(&scope);
    let first_scope: Arc<dyn RequestScopeSource> = concrete;
    assert!(attach_tenant_fan_out(
        &mut registry,
        factories(),
        first_scope,
        &[first_config],
        BTreeSet::new(),
    ));
    let second_scope: Arc<dyn RequestScopeSource> = scope;
    assert!(!attach_tenant_fan_out(
        &mut registry,
        factories(),
        second_scope,
        &[second_config],
        BTreeSet::from(["postgresql".to_owned()]),
    ));
    assert!(registry.fan_out.is_some());
}
