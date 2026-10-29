#![cfg(test)]

use crate::entry::{ModelFieldError, ModelFields};
use rstest::rstest;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[rstest]
#[case::pinned_sources("modelEntry")]
fn model_metadata_types_model_field_tests_behavior(#[case] definition: &str) {
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
    for name in schema.definitions[definition].properties.keys().chain(
        entries
            .iter()
            .filter(|(name, _)| {
                name.as_str() != "sample_spec" && name.as_str() != "fallback_generalizations"
            })
            .flat_map(|(_, entry)| entry.keys()),
    ) {
        if is_price_owned(name) {
            continue;
        }
        assert_field_round_trips(name);
    }
    for (name, entry) in &entries {
        if name == "sample_spec" || name == "fallback_generalizations" {
            continue;
        }
        let raw = RawValue::from_string(serde_json::to_string(entry).unwrap()).unwrap();
        ModelFields::parse_strict(&raw).unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

fn is_price_owned(name: &str) -> bool {
    name.contains("cost")
        || name.contains("multiplier")
        || [
            "provider_specific_entry",
            "off_peak_pricing",
            "tiered_pricing",
        ]
        .contains(&name)
}

fn assert_field_round_trips(name: &str) {
    let sentinel = field_sentinel(name);
    let input = RawValue::from_string(format!("{{{name:?}:{sentinel}}}")).unwrap();
    let (fields, warnings) = ModelFields::parse(&input)
        .unwrap_or_else(|error| panic!("typed field {name} failed to parse: {error}"));
    assert!(warnings.is_empty(), "typed field {name} reported unknown");
    let projected: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(&serde_json::to_string(&fields).unwrap()).unwrap();
    assert!(
        projected.contains_key(name),
        "missing typed field {name} in projection"
    );
}

// A boolean sentinel fits every capability flag; the metadata fields that
// are not booleans are covered by name here since a numeric/string/array
// sentinel would need a per-field table for one inventory sweep.
fn field_sentinel(name: &str) -> &'static str {
    match name {
        "litellm_provider" | "audio_transcription_config" | "source" | "comment" => r#""sentinel""#,
        "mode" => "\"chat\"",
        "max_input_tokens"
        | "max_output_tokens"
        | "max_tokens"
        | "rpm"
        | "tpm"
        | "prompt_cache_min_tokens"
        | "output_vector_size" => "1",
        "supported_endpoints" | "supported_regions" => r#"["sentinel"]"#,
        "supported_modalities" | "supported_output_modalities" => r#"["text"]"#,
        "supported_audio_formats" => r#"["mp3"]"#,
        "default_reasoning_effort" | "bedrock_output_config_effort_ceiling" => "\"low\"",
        "reasoning_effort_levels" => r#"["low"]"#,
        "vertex_ai_audio_api" => "\"lyria_predict\"",
        "web_search_billing_unit" => "\"per_query\"",
        "deprecation_date" => "\"2025-06-30\"",
        "metadata" => r#"{"successor":"sentinel"}"#,
        _ => "true",
    }
}

#[test]
fn strict_parsing_rejects_a_field_the_pinned_catalog_has_never_carried() {
    let raw: Box<RawValue> = serde_json::from_str(
        r#"{"litellm_provider":"openai","a_field_the_catalog_has_never_carried":true}"#,
    )
    .unwrap();
    let error = ModelFields::parse_strict(&raw).unwrap_err();
    assert!(
        matches!(error, ModelFieldError::Unknown(path) if path == "a_field_the_catalog_has_never_carried")
    );
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
}
