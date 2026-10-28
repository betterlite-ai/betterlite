#![cfg(test)]

use super::PriceFields;
use rstest::rstest;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[rstest]
#[case::absent("{}", "absent")]
#[case::exact_digits(
    r#"{"input_cost_per_token":0.0000001234567890123456789}"#,
    "0.0000001234567890123456789"
)]
#[case::positive_exponent(r#"{"input_cost_per_token":1.25e2}"#, "125")]
#[case::maximum_scale(r#"{"input_cost_per_token":1e-28}"#, "0.0000000000000000000000000001")]
#[case::underflow(r#"{"input_cost_per_token":1e-29}"#, "input_cost_per_token")]
#[case::secondary_null(r#"{"annotation_cost_per_page":null}"#, "annotation_cost_per_page")]
#[case::nested_precision(
    r#"{"guardrail_cost_per_unit":{"custom":1e-29}}"#,
    "guardrail_cost_per_unit.custom"
)]
#[case::metadata_null(
    r#"{"metadata":{"original_pricing_per_hour":null}}"#,
    "metadata.original_pricing_per_hour"
)]
#[case::search_null(
    r#"{"search_context_cost_per_query":{"search_context_size_low":null}}"#,
    "search_context_cost_per_query.search_context_size_low"
)]
#[case::off_peak_null(
    r#"{"off_peak_pricing":{"input_cost_per_token":null}}"#,
    "off_peak_pricing.input_cost_per_token"
)]
#[case::tier_null(
    r#"{"tiered_pricing":[{"input_cost_per_query":null}]}"#,
    "tiered_pricing[0].input_cost_per_query"
)]
#[case::duplicate_unit(
    r#"{"guardrail_cost_per_unit":{"custom":1,"custom":2}}"#,
    "duplicate key"
)]
#[case::expanded_exponent(r#"{"input_cost_per_token":1.25e3}"#, "1250")]
#[case::expanded_overflow(r#"{"input_cost_per_token":1e29}"#, "input_cost_per_token")]
#[case::unbounded_exponent(
    r#"{"input_cost_per_token":1e999999999999999999999}"#,
    "input_cost_per_token"
)]
#[case::false_boolean(r#"{"input_cost_per_token":false}"#, "must be a JSON number")]
#[case::fraction(r#"{"input_cost_per_token":0.000000123456789}"#, "0.000000123456789")]
#[case::exponent(r#"{"output_cost_per_token":1.2e-7}"#, "0.00000012")]
#[case::overprecision(
    r#"{"input_cost_per_token":0.12345678901234567890123456789}"#,
    "input_cost_per_token"
)]
#[case::overflow(
    r#"{"input_cost_per_token":999999999999999999999999999999999999999999999999}"#,
    "input_cost_per_token"
)]
#[case::overprecision_exponent(
    r#"{"input_cost_per_token":0.12345678901234567890123456789e-1}"#,
    "input_cost_per_token"
)]
#[case::overflow_exponent(
    r#"{"input_cost_per_token":999999999999999999999999999999e10}"#,
    "input_cost_per_token"
)]
#[case::nested(r#"{"provider_specific_entry":{"us":0.000000000000000001},"guardrail_cost_per_unit":{"contentPolicyUnits":0.0000001},"metadata":{"original_pricing_per_minute":0.020833333333333333},"sample_spec":{"computer_use_input_cost_per_1k_tokens":0.00001,"file_search_cost_per_1k_calls":0.003},"off_peak_pricing":{"input_cost_per_token":0.0000002,"windows":[{"hours_utc":"00:00-01:00","weekdays":["monday",2]}]},"search_context_cost_per_query":{"search_context_size_high":0.01},"tiered_pricing":[{"input_cost_per_token":0.00000005,"range":[0,256000]}]}"#, "nested")]
#[case::unknown_tier(r#"{"tiered_pricing":[{"input_cost_per_token":5e-8,"max_results_range":[0,25],"unknown_price":0.5}]}"#, "unknown field `unknown_price`")]
#[case::extreme_exponent(r#"{"input_cost_per_token":1e-2147483648}"#, "input_cost_per_token")]
#[case::nested_null(
    r#"{"provider_specific_entry":{"us":null}}"#,
    "provider_specific_entry.us"
)]
#[case::null(r#"{"input_cost_per_token":null}"#, "must not be null")]
#[case::string(r#"{"input_cost_per_token":"0.1"}"#, "must be a JSON number")]
#[case::boolean(r#"{"input_cost_per_token":true}"#, "must be a JSON number")]
#[case::array(r#"{"input_cost_per_token":[]}"#, "must be a JSON number")]
#[case::object(r#"{"input_cost_per_token":{}}"#, "must be a JSON number")]
fn model_price_types_price_field_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let raw: Result<Box<RawValue>, _> = serde_json::from_str(source);
    let parsed = match raw {
        Ok(value) => PriceFields::parse(value.as_ref()),
        Err(error) => panic!("test JSON must parse: {error}"),
    };
    if expected == "absent" {
        assert_eq!(parsed.unwrap(), PriceFields::default());
    } else if expected == "nested" {
        let parsed_value = parsed.unwrap();
        assert_eq!(
            parsed_value.provider_specific.us.unwrap().to_string(),
            "0.000000000000000001"
        );
        assert_eq!(parsed_value.guardrail_units.len(), 1);
        assert_eq!(
            parsed_value.guardrail_units["contentPolicyUnits"].to_string(),
            "0.0000001"
        );
        assert_eq!(
            parsed_value.metadata.per_minute.unwrap().to_string(),
            "0.020833333333333333"
        );
        assert_eq!(
            parsed_value
                .sample_spec
                .computer_use_input
                .unwrap()
                .to_string(),
            "0.00001"
        );
        assert_eq!(
            parsed_value
                .sample_spec
                .file_search_calls
                .unwrap()
                .to_string(),
            "0.003"
        );
        assert_eq!(
            parsed_value
                .off_peak
                .unwrap()
                .input_cost_per_token
                .unwrap()
                .to_string(),
            "0.0000002"
        );
        assert_eq!(
            parsed_value
                .search_context
                .unwrap()
                .search_context_size_high
                .unwrap()
                .to_string(),
            "0.01"
        );
        assert_eq!(parsed_value.tiers.len(), 1);
        assert_eq!(parsed_value.tiers[0].range.as_ref().unwrap().len(), 2);
        assert_eq!(
            parsed_value.tiers[0].range.as_ref().unwrap()[1].to_string(),
            "256000"
        );
    } else if expected.contains('_') || expected.contains("must") || expected.contains("duplicate")
    {
        assert!(parsed.unwrap_err().to_string().contains(expected));
    } else {
        let parsed_value = parsed.unwrap();
        assert_eq!(
            parsed_value
                .values
                .input_token
                .or(parsed_value.values.output_token)
                .unwrap()
                .to_string(),
            expected
        );
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

mod inventory {
    use super::{PriceFields, Schema, assert_prices, assert_schema};
    use rstest::rstest;
    use serde_json::value::RawValue;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    #[rstest]
    #[case::pinned_sources("modelEntry")]
    fn model_price_types_price_field_tests_behavior(#[case] definition: &str) {
        let schema: Schema = serde_json::from_str(include_str!(
            "../../fixtures/model_prices_and_context_window.schema.json"
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
                if ["guardrail_cost_per_unit", "search_context_cost_per_query"]
                    .contains(&name.as_str())
                {
                    continue;
                }
                let input =
                    RawValue::from_string(format!("{{{name:?}:0.0000001234567890123456789}}"))
                        .unwrap();
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
                let projected =
                    RawValue::from_string(serde_json::to_string(&prices).unwrap()).unwrap();
                assert_prices(&raw, Some(&projected), "");
            }
        }
    }
}

#[rstest]
fn replays_the_price_fields_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/price_fields");
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let source = fs::read_to_string(path).unwrap();
        let raw: Box<RawValue> = serde_json::from_str(&source).unwrap();
        drop(PriceFields::parse(raw.as_ref()));
    }
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
    let expected = super::parse_decimal_lexeme(source.get())
        .unwrap()
        .normalize()
        .to_string();
    let actual: String =
        serde_json::from_str(projected.unwrap_or_else(|| panic!("missing {path}")).get()).unwrap();
    assert_eq!(
        super::parse_decimal_lexeme(&actual)
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
