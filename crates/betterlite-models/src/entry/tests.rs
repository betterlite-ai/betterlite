#![cfg(test)]

use super::generalizations::FallbackGeneralizations;
use super::{ModelFieldError, ModelFields};
use crate::metadata::identity::Mode;
use rstest::rstest;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[rstest]
#[case::absent("{}", "absent")]
#[case::limits(
    r#"{"litellm_provider":"openai","max_input_tokens":128000,"max_output_tokens":16384,"rpm":10000}"#,
    "ok"
)]
#[case::omitted_flag_is_not_false(r#"{"litellm_provider":"openai"}"#, "omitted")]
#[case::explicit_false_flag(r#"{"litellm_provider":"openai","supports_vision":false}"#, "false")]
#[case::modalities_and_regions(
    r#"{"litellm_provider":"vertex_ai","supported_modalities":["text","image"],"supported_output_modalities":["text","code"],"supported_regions":["global"]}"#,
    "ok"
)]
#[case::provider_mode(r#"{"litellm_provider":"anthropic","mode":"chat"}"#, "ok")]
#[case::duplicate_known_field(
    r#"{"litellm_provider":"openai","mode":"chat","mode":"embedding"}"#,
    "duplicate field"
)]
#[case::unknown_enum_value(r#"{"litellm_provider":"openai","mode":"not_a_real_mode"}"#, "mode")]
fn model_metadata_types_model_field_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let raw: Box<RawValue> = serde_json::from_str(source).unwrap();
    let parsed = ModelFields::parse(raw.as_ref());
    match expected {
        "absent" => {
            let (fields, warnings) = parsed.unwrap();
            assert_eq!(fields, ModelFields::default());
            assert!(warnings.is_empty());
        }
        "ok" => {
            parsed.unwrap();
        }
        "omitted" => {
            let (fields, _) = parsed.unwrap();
            assert_eq!(fields.capabilities.media.supports_vision, None);
        }
        "false" => {
            let (fields, _) = parsed.unwrap();
            assert_eq!(fields.capabilities.media.supports_vision, Some(false));
        }
        _ => {
            let error = parsed.unwrap_err();
            assert!(error.to_string().contains(expected));
        }
    }
}

#[test]
fn runtime_parsing_accepts_an_unknown_field_and_warns_with_its_path() {
    let raw: Box<RawValue> = serde_json::from_str(
        r#"{"litellm_provider":"openai","mode":"chat","totally_new_field":true,"metadata":{"brand_new_note":"x"}}"#,
    )
    .unwrap();
    let (fields, warnings) = ModelFields::parse(raw.as_ref()).unwrap();
    assert_eq!(fields.metadata.litellm_provider, "openai");
    assert!(warnings.contains(&"totally_new_field".to_owned()));
    assert!(warnings.contains(&"metadata.brand_new_note".to_owned()));
}

#[test]
fn strict_parsing_rejects_the_same_unknown_field() {
    let raw: Box<RawValue> =
        serde_json::from_str(r#"{"litellm_provider":"openai","totally_new_field":true}"#).unwrap();
    let error = ModelFields::parse_strict(raw.as_ref()).unwrap_err();
    assert!(matches!(error, ModelFieldError::Unknown(path) if path == "totally_new_field"));
}

#[test]
fn strict_parsing_does_not_treat_b204s_price_fields_as_unknown() {
    let raw: Box<RawValue> = serde_json::from_str(
        r#"{"litellm_provider":"openai","input_cost_per_token":0.0001,"tiered_pricing":[{"input_cost_per_token":0.00005}]}"#,
    )
    .unwrap();
    ModelFields::parse_strict(raw.as_ref()).unwrap();
}

#[test]
fn documentation_and_fallback_sentinel_entries_are_excluded_from_model_enumeration() {
    let entries: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/model_prices_and_context_window.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(entries.contains_key("sample_spec"));
    assert!(entries.contains_key("fallback_generalizations"));
    let models: Vec<&String> = entries
        .keys()
        .filter(|name| {
            name.as_str() != "sample_spec" && name.as_str() != "fallback_generalizations"
        })
        .collect();
    assert!(!models.is_empty());
    assert!(!models.contains(&&"sample_spec".to_owned()));
}

#[test]
fn the_documentation_sample_entry_fails_typed_parsing() {
    let entries: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/model_prices_and_context_window.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let sample = &entries["sample_spec"];
    ModelFields::parse(sample.as_ref()).unwrap_err();
}

#[test]
fn fallback_generalization_rules_type_their_declaration_without_executing_it() {
    let entries: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/model_prices_and_context_window.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let raw = &entries["fallback_generalizations"];
    let generalizations: FallbackGeneralizations = serde_json::from_str(raw.get()).unwrap();
    assert!(!generalizations.rules.is_empty());
    let named = generalizations
        .rules
        .iter()
        .find(|rule| rule.name == "bedrock-claude-ids")
        .unwrap();
    assert_eq!(
        named.model_info.metadata.litellm_provider,
        "bedrock".to_owned()
    );
}

#[test]
fn provider_projection_exposes_provider_and_mode_without_a_provider_crate() {
    let raw: Box<RawValue> =
        serde_json::from_str(r#"{"litellm_provider":"anthropic","mode":"chat"}"#).unwrap();
    let (fields, _) = ModelFields::parse(raw.as_ref()).unwrap();
    let projection = fields.provider_projection();
    assert_eq!(projection.litellm_provider, "anthropic");
    assert_eq!(projection.mode, Some(Mode::Chat));
}

#[path = "inventory/tests.rs"]
mod inventory;

#[test]
fn a_flattened_fields_bad_enum_value_recovers_its_own_path_not_the_lost_top_level_dot() {
    let raw: Box<RawValue> =
        serde_json::from_str(r#"{"litellm_provider":"openai","mode":"not_a_real_mode"}"#).unwrap();
    let error = ModelFields::parse(raw.as_ref()).unwrap_err();
    let ModelFieldError::Json { path, .. } = error else {
        panic!("expected a Json error, got {error:?}");
    };
    assert_eq!(path, "mode");
}

#[rstest]
fn replays_the_model_fields_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/model_fields");
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let source = fs::read_to_string(path).unwrap();
        let Ok(raw) = serde_json::from_str::<Box<RawValue>>(&source) else {
            continue;
        };
        drop(ModelFields::parse(raw.as_ref()));
    }
}
