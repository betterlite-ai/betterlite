#![cfg(test)]

use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::mappers::base::AttributeTable;
use crate::otel::mappers::utils::{
    MAX_TOOL_ATTRS_PER_SPAN, collect, drop_none, drop_none_pairs, json_if, tool_attr_budget,
    tool_definition_attrs,
};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::tool_definition::ToolDefinition;
use crate::otel::model::utils::coercion::OtelModelValue;

const NAMES: AttributeTable<ToolDefinition> =
    &[("name", |tool| Some(Value::from(tool.name.clone())))];

const NAMES_AND_DESCRIPTIONS: AttributeTable<ToolDefinition> = &[
    ("name", |tool| Some(Value::from(tool.name.clone()))),
    ("description", |tool| {
        tool.description.clone().map(Value::from)
    }),
];

fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: None,
        parameters_json: None,
    }
}

#[rstest::rstest]
#[case::no_vocabularies(0, MAX_TOOL_ATTRS_PER_SPAN)]
#[case::one_vocabulary(1, MAX_TOOL_ATTRS_PER_SPAN)]
#[case::four_vocabularies(4, 8)]
#[case::more_vocabularies_than_attrs(64, 0)]
fn tool_budget(#[case] vocabularies: usize, #[case] expected: usize) {
    assert_eq!(tool_attr_budget(vocabularies), expected);
}

#[test]
fn tool_definitions_keep_one_tool_without_a_budget() {
    let tools = [tool("first"), tool("second")];
    let attributes = tool_definition_attrs("gen_ai.tool.", &tools, NAMES, 0);
    assert_eq!(
        attributes.get("gen_ai.tool.0.name"),
        Some(&Value::from("first"))
    );
    assert!(!attributes.contains_key("gen_ai.tool.1.name"));
}

#[test]
fn tool_definitions_share_the_budget_across_fields() {
    let tools = [tool("first"), tool("second"), tool("third")];
    let attributes =
        tool_definition_attrs("llm.request.functions.", &tools, NAMES_AND_DESCRIPTIONS, 4);
    assert!(attributes.contains_key("llm.request.functions.0.name"));
    assert!(attributes.contains_key("llm.request.functions.1.name"));
    assert!(!attributes.contains_key("llm.request.functions.2.name"));
}

#[test]
fn collect_drops_the_absent_values() {
    const MIXED: AttributeTable<ToolDefinition> = &[
        ("name", |tool| Some(Value::from(tool.name.clone()))),
        ("description", |_tool| None),
    ];
    let attributes = collect(MIXED, &tool("named"));
    assert_eq!(attributes.len(), 1);
    assert!(attributes.contains_key("name"));
}

#[test]
fn drop_none_pairs_removes_the_absent_values() {
    let attributes =
        drop_none_pairs([("kept", Some(Value::from("value"))), ("dropped", None)].into_iter());
    assert_eq!(attributes.len(), 1);
    assert_eq!(attributes.get("kept"), Some(&Value::from("value")));
}

#[test]
fn drop_none_removes_the_absent_values() {
    let values = BTreeMap::from([
        ("kept".to_owned(), Some(Value::from("value"))),
        ("dropped".to_owned(), None),
    ]);
    let attributes = drop_none(&values);
    assert_eq!(attributes.len(), 1);
}

#[test]
fn json_if_skips_the_empty_payload() {
    assert_eq!(json_if(&MetadataMap::new()), None);
    let payload =
        MetadataMap::from([("key".to_owned(), OtelModelValue::String("value".to_owned()))]);
    assert_eq!(json_if(&payload), Some(r#"{"key":"value"}"#.to_owned()));
}
