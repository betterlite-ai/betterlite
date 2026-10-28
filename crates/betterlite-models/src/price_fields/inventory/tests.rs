#![cfg(test)]

use crate::price_fields::{PriceFields, parse_decimal_lexeme};
use rstest::rstest;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[rstest]
#[case::pinned_sources("modelEntry")]
fn model_price_types_price_field_tests_behavior(#[case] definition: &str) {
    let schema: Schema = serde_json::from_str(include_str!(
        "../../../fixtures/model_prices_and_context_window.schema.json"
    ))
    .unwrap();
    let entries: BTreeMap<String, BTreeMap<String, Box<RawValue>>> = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/model_prices_and_context_window.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_schema(&schema.definitions[definition], &[]);
    for name in schema.definitions[definition]
        .properties
        .keys()
        .chain(entries.values().flat_map(BTreeMap::keys))
    {
        if name.contains("cost") || name.contains("multiplier") {
            if ["guardrail_cost_per_unit", "search_context_cost_per_query"].contains(&name.as_str())
            {
                continue;
            }
            let input =
                RawValue::from_string(format!("{{{name:?}:0.0000001234567890123456789}}")).unwrap();
            let prices = PriceFields::parse(&input).unwrap();
            let output: BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(&serde_json::to_string(&prices).unwrap()).unwrap();
            assert_eq!(
                output.get(name).map(|value| value.get()),
                Some("\"0.0000001234567890123456789\""),
                "missing typed field {name}"
            );
        }
    }
    for (name, entry) in entries {
        if name != "sample_spec" && name != "fallback_generalizations" {
            let raw = RawValue::from_string(serde_json::to_string(&entry).unwrap()).unwrap();
            let prices = PriceFields::parse(&raw).unwrap();
            let projected = RawValue::from_string(serde_json::to_string(&prices).unwrap()).unwrap();
            assert_prices(&raw, Some(&projected), "");
        }
    }
}
#[derive(Deserialize)]
struct Schema {
    #[serde(rename = "$defs")]
    definitions: BTreeMap<String, Definition>,
}

#[derive(Deserialize)]
struct Definition {
    #[serde(default)]
    properties: BTreeMap<String, Self>,
    items: Option<Box<Self>>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn assert_prices(source: &RawValue, projected: Option<&RawValue>, path: &str) {
    if source.get().starts_with('{') {
        let fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(source.get()).unwrap();
        let output: BTreeMap<String, Box<RawValue>> = projected
            .map(|raw| serde_json::from_str(raw.get()).unwrap())
            .unwrap_or_default();
        for (key, value) in fields {
            assert_prices(
                &value,
                output.get(&key).map(Box::as_ref),
                &format!("{path}.{key}"),
            );
        }
    } else if source.get().starts_with('[') {
        let items: Vec<Box<RawValue>> = serde_json::from_str(source.get()).unwrap();
        let output: Vec<Box<RawValue>> = projected
            .map(|raw| serde_json::from_str(raw.get()).unwrap())
            .unwrap_or_default();
        for (index, value) in items.iter().enumerate() {
            assert_prices(
                value,
                output.get(index).map(Box::as_ref),
                &format!("{path}[{index}]"),
            );
        }
    } else if (path.contains("cost")
        || path.contains("multiplier")
        || path.contains("original_pricing")
        || path.ends_with("provider_specific_entry.us")
        || path.ends_with("provider_specific_entry.fast"))
        && !source.get().starts_with('"')
    {
        assert_scalar(source, projected, path);
    } else {
        assert!(!path.is_empty());
    }
}

fn assert_scalar(source: &RawValue, projected: Option<&RawValue>, path: &str) {
    let expected = parse_decimal_lexeme(source.get())
        .unwrap()
        .normalize()
        .to_string();
    let actual: String =
        serde_json::from_str(projected.unwrap_or_else(|| panic!("missing {path}")).get()).unwrap();
    assert_eq!(
        parse_decimal_lexeme(&actual)
            .unwrap()
            .normalize()
            .to_string(),
        expected,
        "{path}"
    );
}

fn assert_schema(schema: &Definition, path: &[String]) {
    if schema.kind.as_deref() == Some("number")
        && path
            .iter()
            .any(|key| key.contains("cost") || key.contains("multiplier"))
    {
        let mut source = "0.0000001234567890123456789".to_owned();
        for key in path.iter().rev() {
            source = if key == "[]" {
                format!("[{source}]")
            } else {
                format!("{{{key:?}:{source}}}")
            };
        }
        let raw = RawValue::from_string(source).unwrap();
        let prices = PriceFields::parse(&raw).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        let projected = RawValue::from_string(serde_json::to_string(&prices).unwrap()).unwrap();
        assert_prices(&raw, Some(&projected), "");
    }
    for (name, nested) in &schema.properties {
        let mut child = path.to_vec();
        child.push(name.clone());
        assert_schema(nested, &child);
    }
    if let Some(items) = &schema.items {
        let mut child = path.to_vec();
        child.push("[]".to_owned());
        assert_schema(items, &child);
    }
}
