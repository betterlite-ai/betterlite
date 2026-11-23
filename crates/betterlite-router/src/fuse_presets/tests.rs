#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;

use super::error::FusePresetsError;
use super::{
    FuseModelPreset, FusePresetCatalog, MAX_PROFILE_TEXT_CHARACTERS, PresetKind, ProfileText,
    get_fuse_presets, resolve_fuse_profile,
};

const ASTRA_TEXT: &str = "OpenAI model for demanding end-to-end work, including reasoning, coding, research, and document tasks";

const CLAUDE_CODE_TEXT: &str = "Claude Code supplies an agent loop with context management and configured tools. Available actions depend on the session's tools, permissions, and execution environment. The runtime name alone does not establish repository access, runnable tests, network access, additional tools, or a step, time, or spending budget";

#[test]
fn catalog_bundled_file_holds_the_version_and_ids_in_file_order() {
    let catalog = get_fuse_presets().unwrap();
    assert_eq!(catalog.version, "2026-09-22-v1");
    let models: Vec<&str> = catalog
        .models
        .iter()
        .map(|preset| preset.id.as_str())
        .collect();
    assert_eq!(
        models.as_slice(),
        [
            "gpt-6-astra-v1",
            "gpt-6-sol-v1",
            "gpt-6-luna-v1",
            "gpt-5.6-sol-v1",
            "gpt-5.6-luna-v1",
            "gpt-5.6-terra-v1",
            "claude-haiku-4-5-v1",
            "claude-sonnet-5-v1",
            "claude-opus-5-v1",
            "claude-fable-5-v1",
            "claude-fable-5-1-v1",
            "claude-opus-5-5-v1",
        ]
    );
    let harnesses: Vec<&str> = catalog
        .harnesses
        .iter()
        .map(|preset| preset.id.as_str())
        .collect();
    assert_eq!(
        harnesses.as_slice(),
        [
            "unspecified-v1",
            "claude-code-v1",
            "codex-cli-v1",
            "opencode-v1",
            "mini-swe-agent-v1",
        ]
    );
}

#[test]
fn catalog_bundled_file_carries_each_preset_text_and_source() {
    let catalog = get_fuse_presets().unwrap();
    assert_eq!(catalog.models[0].label, "GPT-6 Astra");
    assert_eq!(catalog.models[0].text.to_string(), ASTRA_TEXT);
    let astra: &str = catalog.models[0].text.as_ref();
    assert_eq!(astra, ASTRA_TEXT);
    assert_eq!(catalog.models[0].model, "gpt-6-astra");
    assert_eq!(
        catalog.models[0].sources.as_slice(),
        ["https://developers.openai.com/api/docs/models/gpt-6-astra"]
    );
    assert_eq!(catalog.harnesses[1].text.to_string(), CLAUDE_CODE_TEXT);
    assert_eq!(
        catalog.harnesses[1].sources.as_slice(),
        ["https://code.claude.com/docs/en/how-claude-code-works"]
    );
}

#[test]
fn catalog_is_parsed_once_and_shared() {
    let first = get_fuse_presets().unwrap();
    let second = get_fuse_presets().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

#[rstest]
#[case::unknown_catalog_field(
    r#"{"version":"v","models":[],"harnesses":[],"extra":true}"#,
    "unknown field"
)]
#[case::unknown_preset_field(
    r#"{"version":"v","models":[{"id":"a","label":"A","text":"t","sources":["s"],"model":"m","extra":true}],"harnesses":[]}"#,
    "unknown field"
)]
#[case::unknown_harness_field(
    r#"{"version":"v","models":[],"harnesses":[{"id":"h","label":"H","text":"t","sources":["s"],"extra":true}]}"#,
    "unknown field"
)]
#[case::empty_model_sources(
    r#"{"version":"v","models":[{"id":"a","label":"A","text":"t","sources":[],"model":"m"}],"harnesses":[]}"#,
    "a fuse preset needs at least one source"
)]
#[case::empty_harness_sources(
    r#"{"version":"v","models":[],"harnesses":[{"id":"h","label":"H","text":"t","sources":[]}]}"#,
    "a fuse preset needs at least one source"
)]
fn catalog_rejects_unknown_fields_and_empty_sources(#[case] raw: &str, #[case] reason: &str) {
    let error = serde_json::from_str::<FusePresetCatalog>(raw).unwrap_err();
    assert!(error.to_string().contains(reason));
}

#[test]
fn catalog_model_preset_serializes_keys_in_declaration_order() {
    let preset = FuseModelPreset {
        id: "gpt-6-astra-v1".to_owned(),
        label: "GPT-6 Astra".to_owned(),
        text: ProfileText::try_from(ASTRA_TEXT.to_owned()).unwrap(),
        sources: vec!["https://developers.openai.com/api/docs/models/gpt-6-astra".to_owned()],
        model: "gpt-6-astra".to_owned(),
    };
    assert_eq!(
        serde_json::to_string(&preset).unwrap(),
        r#"{"id":"gpt-6-astra-v1","label":"GPT-6 Astra","text":"OpenAI model for demanding end-to-end work, including reasoning, coding, research, and document tasks","sources":["https://developers.openai.com/api/docs/models/gpt-6-astra"],"model":"gpt-6-astra"}"#
    );
}

#[rstest]
#[case::no_preset_without_text(None, None, PresetKind::Model, None)]
#[case::no_preset_keeps_text(
    Some("custom profile"),
    None,
    PresetKind::Model,
    Some("custom profile")
)]
#[case::known_model_preset_without_text(
    None,
    Some("gpt-6-astra-v1"),
    PresetKind::Model,
    Some(ASTRA_TEXT)
)]
#[case::known_model_preset_keeps_text(
    Some("custom profile"),
    Some("gpt-6-astra-v1"),
    PresetKind::Model,
    Some("custom profile")
)]
#[case::known_harness_preset_without_text(
    None,
    Some("claude-code-v1"),
    PresetKind::Harness,
    Some(CLAUDE_CODE_TEXT)
)]
#[case::known_harness_preset_keeps_text(
    Some("custom profile"),
    Some("claude-code-v1"),
    PresetKind::Harness,
    Some("custom profile")
)]
#[case::unknown_preset_ignores_text(
    Some("custom profile"),
    Some("missing-v1"),
    PresetKind::Model,
    None
)]
#[case::model_id_asked_as_harness(None, Some("gpt-6-astra-v1"), PresetKind::Harness, None)]
#[case::harness_id_asked_as_model(None, Some("claude-code-v1"), PresetKind::Model, None)]
fn resolve_fuse_profile_returns_text_or_preset_text(
    #[case] text: Option<&str>,
    #[case] preset_id: Option<&str>,
    #[case] kind: PresetKind,
    #[case] expected: Option<&str>,
) {
    let profile = text.map(|value| ProfileText::try_from(value.to_owned()).unwrap());
    let resolved = resolve_fuse_profile(profile.as_ref(), preset_id, kind).unwrap();
    assert_eq!(
        resolved.map(|value| value.to_string()),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::spaces("  centered text  ", "centered text")]
#[case::tabs("\tprofile\t", "profile")]
#[case::newlines("\nprofile\n", "profile")]
#[case::line_breaks("\r\n profile \r\n", "profile")]
fn profile_text_strips_surrounding_whitespace(#[case] raw: &str, #[case] stripped: &str) {
    let text = ProfileText::try_from(raw.to_owned()).unwrap();
    assert_eq!(text.to_string(), stripped);
    assert_eq!(text.as_ref(), stripped);
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::mixed(" \t\r\n ")]
fn profile_text_whitespace_only_is_too_short(#[case] raw: &str) {
    let error = ProfileText::try_from(raw.to_owned()).unwrap_err();
    assert!(matches!(error, FusePresetsError::TextTooShort));
}

#[test]
fn profile_text_accepts_exactly_4000_characters_after_stripping() {
    let raw = format!("  {}  ", "x".repeat(MAX_PROFILE_TEXT_CHARACTERS));
    let text = ProfileText::try_from(raw).unwrap();
    assert_eq!(text.to_string().chars().count(), 4000);
}

#[test]
fn profile_text_rejects_4001_characters() {
    let raw = "x".repeat(MAX_PROFILE_TEXT_CHARACTERS + 1);
    let error = ProfileText::try_from(raw).unwrap_err();
    assert!(matches!(
        error,
        FusePresetsError::TextTooLong {
            length: 4001,
            limit: 4000
        }
    ));
}

#[test]
fn profile_text_counts_characters_not_bytes() {
    let raw = format!("\t{}\n", "é".repeat(MAX_PROFILE_TEXT_CHARACTERS));
    let text = ProfileText::try_from(raw).unwrap();
    let rendered = text.to_string();
    assert_eq!(rendered.chars().count(), 4000);
    assert_eq!(rendered.len(), 8000);
}
