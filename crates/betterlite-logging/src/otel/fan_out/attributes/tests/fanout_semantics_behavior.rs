#![cfg(test)]

use std::collections::BTreeSet;

use opentelemetry::Value;
use rstest::rstest;

use crate::otel::config::destination::SpanScope;
use crate::otel::fan_out::attributes::{
    attribute_map, in_scope, is_excluded_database_span, is_llm_call_span, tenant_view,
};
use crate::otel::fan_out::tests::support::{attributes, keys, root_span, string_attribute};

fn excluded(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[rstest]
#[case::name_convention("db.system.name", "postgresql", &["postgresql"], true)]
#[case::legacy_convention("db.system", "postgresql", &["postgresql"], true)]
#[case::other_system("db.system.name", "mysql", &["postgresql"], false)]
#[case::no_exclusions("db.system.name", "postgresql", &[], false)]
fn excluded_database_spans_name_the_configured_system(
    #[case] key: &str,
    #[case] system: &str,
    #[case] excluded_systems: &[&str],
    #[case] excluded_span: bool,
) {
    let span = root_span(vec![string_attribute(key, system)]);
    let map = attribute_map(&span.attributes);
    assert_eq!(
        is_excluded_database_span(&map, &excluded(excluded_systems)),
        excluded_span
    );
}

#[rstest]
#[case::numeric_system(Value::I64(7))]
#[case::boolean_system(Value::Bool(true))]
fn a_database_system_of_another_type_is_never_excluded(#[case] system: Value) {
    let span = root_span(attributes(&[("db.system.name", system)]));
    let map = attribute_map(&span.attributes);
    assert!(!is_excluded_database_span(&map, &excluded(&["7", "true"])));
}

#[test]
fn a_span_without_a_database_system_is_not_an_excluded_database_span() {
    let span = root_span(vec![string_attribute("server.address", "db.internal")]);
    let map = attribute_map(&span.attributes);
    assert!(!is_excluded_database_span(&map, &excluded(&["postgresql"])));
}

#[test]
fn cache_and_reasoning_token_attributes_reach_the_tenant() {
    let mut span = root_span(vec![
        string_attribute("gen_ai.operation.name", "chat"),
        string_attribute("gen_ai.usage.cache_read.input_tokens", "12"),
    ]);
    span.attributes.push(string_attribute(
        "gen_ai.usage.cache_creation.input_tokens",
        "4",
    ));
    span.attributes
        .push(string_attribute("gen_ai.usage.output_tokens", "9"));
    let view = tenant_view(&span);
    let kept = keys(&view);
    assert!(kept.contains("gen_ai.usage.cache_read.input_tokens"));
    assert!(kept.contains("gen_ai.usage.cache_creation.input_tokens"));
    assert!(kept.contains("gen_ai.usage.output_tokens"));
}

#[rstest]
#[case::llm_operation("gen_ai.operation.name")]
#[case::mcp_tool("mcp.method.name")]
#[case::guardrail("litellm.guardrail.name")]
fn the_tenant_owned_markers_are_the_ones_litellm_names(#[case] marker: &str) {
    let span = root_span(vec![
        string_attribute(marker, "x"),
        string_attribute("error.message", "tenant call failed"),
    ]);
    let view = tenant_view(&span);
    assert!(keys(&view).contains("error.message"));
}

#[test]
fn a_span_with_only_proxy_markers_does_not_count_as_a_model_call() {
    let span = root_span(vec![
        string_attribute("mcp.method.name", "tools/call"),
        string_attribute("litellm.guardrail.name", "pii"),
    ]);
    assert!(!is_llm_call_span(&span));
    assert!(!in_scope(&span, SpanScope::LlmOnly));
}

#[test]
fn a_model_call_with_a_tool_name_is_not_an_mcp_span() {
    let span = root_span(vec![
        string_attribute("gen_ai.operation.name", "chat"),
        string_attribute("gen_ai.tool.name", "search"),
    ]);
    assert!(is_llm_call_span(&span));
    assert!(in_scope(&span, SpanScope::LlmOnly));
}
