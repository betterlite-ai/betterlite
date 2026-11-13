#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::audio::{AudioParam, map_audio_params};
use super::audio::{ResponseModality, map_response_modalities};
use super::effort::{map_effort_to_budget, map_effort_to_level, supports_minimal_level};
use super::error::GeminiThinkingError;
use super::param::{ThinkingParam, ThinkingType, map_thinking_param, validate_thinking_conflicts};
use super::server_flags::apply_server_tool_flags;
use super::settings::{
    MinimalEffortSupport, ThinkingBudgets, ThinkingConfig, ThinkingLevel, ThinkingSettings,
};

const BUDGETS: ThinkingBudgets = ThinkingBudgets {
    minimal: 128,
    minimal_flash: 1,
    minimal_pro: 128,
    minimal_flash_lite: 512,
    low: 1024,
    medium: 2048,
    high: 4096,
    disable: 0,
};

const CUSTOM: ThinkingBudgets = ThinkingBudgets {
    minimal: 10,
    minimal_flash: 11,
    minimal_pro: 12,
    minimal_flash_lite: 13,
    low: 14,
    medium: 15,
    high: 16,
    disable: 17,
};

const SETTINGS: ThinkingSettings = ThinkingSettings {
    budgets: BUDGETS,
    default_thinking_level_low: false,
};

const SETTINGS_LOW: ThinkingSettings = ThinkingSettings {
    budgets: BUDGETS,
    default_thinking_level_low: true,
};

#[derive(Debug)]
struct Supported;

impl MinimalEffortSupport for Supported {
    fn disables_minimal(&self, _model: &str) -> bool {
        false
    }
}

#[derive(Debug)]
struct OptedOut;

impl MinimalEffortSupport for OptedOut {
    fn disables_minimal(&self, _model: &str) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug)]
enum Support {
    Supported,
    OptedOut,
}

impl Support {
    fn model_support(self) -> &'static dyn MinimalEffortSupport {
        match self {
            Self::Supported => &Supported,
            Self::OptedOut => &OptedOut,
        }
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the fixture literal is a JSON object")
}

fn budget_config(thinking_budget: i64, include_thoughts: bool) -> ThinkingConfig {
    ThinkingConfig {
        include_thoughts: Some(include_thoughts),
        thinking_budget: Some(thinking_budget),
        thinking_level: None,
    }
}

fn level_config(thinking_level: ThinkingLevel, include_thoughts: bool) -> ThinkingConfig {
    ThinkingConfig {
        include_thoughts: Some(include_thoughts),
        thinking_budget: None,
        thinking_level: Some(thinking_level),
    }
}

#[rstest]
#[case::flash_lite("gemini-2.5-flash-lite", 512)]
#[case::flash_lite_upper("GEMINI-2.5-FLASH-LITE-PREVIEW", 512)]
#[case::pro("gemini-2.5-pro", 128)]
#[case::pro_preview("gemini-2.5-pro-preview-06-05", 128)]
#[case::flash("gemini-2.5-flash", 1)]
#[case::generic("gemini-2.0-flash", 128)]
#[case::gemini_3("gemini-3-pro-preview", 128)]
fn minimal_effort_uses_the_model_minimum(#[case] model: &str, #[case] expected: i64) {
    let mapped =
        map_effort_to_budget("minimal", Some(model), &BUDGETS).expect("minimal is supported");

    assert_eq!(mapped, budget_config(expected, true));
}

#[test]
fn minimal_effort_without_a_model_uses_the_generic_minimum() {
    let mapped = map_effort_to_budget("minimal", None, &BUDGETS).expect("minimal is supported");

    assert_eq!(mapped, budget_config(128, true));
}

#[rstest]
#[case::low("low", 14, true)]
#[case::medium("medium", 15, true)]
#[case::high("high", 16, true)]
#[case::disable("disable", 17, false)]
#[case::none("none", 0, false)]
fn budget_efforts_read_the_configured_budgets(
    #[case] effort: &str,
    #[case] expected: i64,
    #[case] include_thoughts: bool,
) {
    let mapped =
        map_effort_to_budget(effort, None, &CUSTOM).expect("every effort here is supported");

    assert_eq!(mapped, budget_config(expected, include_thoughts));
}

#[test]
fn an_unsupported_effort_is_a_400() {
    let error = map_effort_to_budget("extreme", Some("gemini-2.5-flash"), &BUDGETS)
        .expect_err("extreme is not a supported effort");

    assert_eq!(error.status(), Some(400));
    assert!(matches!(
        error,
        GeminiThinkingError::UnsupportedEffort { .. }
    ));
}

#[rstest]
#[case::minimal_supported("minimal", Some("gemini-3-flash"), &Supported, ThinkingLevel::Minimal, true)]
#[case::minimal_opted_out("minimal", Some("gemini-3-flash"), &OptedOut, ThinkingLevel::Low, true)]
#[case::minimal_not_flash("minimal", Some("gemini-3-pro"), &Supported, ThinkingLevel::Low, true)]
#[case::minimal_older_model("minimal", Some("gemini-2.5-flash"), &Supported, ThinkingLevel::Low, true)]
#[case::minimal_without_model("minimal", None, &Supported, ThinkingLevel::Low, true)]
#[case::low("low", Some("gemini-3-pro"), &Supported, ThinkingLevel::Low, true)]
#[case::medium("medium", Some("gemini-3-pro"), &Supported, ThinkingLevel::Medium, true)]
#[case::high("high", Some("gemini-3-pro"), &Supported, ThinkingLevel::High, true)]
#[case::disable_supported("disable", Some("gemini-3-flash"), &Supported, ThinkingLevel::Minimal, false)]
#[case::disable_opted_out("disable", Some("gemini-3-flash"), &OptedOut, ThinkingLevel::Low, false)]
#[case::none_supported("none", Some("gemini-3-flash"), &Supported, ThinkingLevel::Minimal, false)]
fn level_efforts_pick_the_minimal_fallback(
    #[case] effort: &str,
    #[case] model: Option<&str>,
    #[case] support: &dyn MinimalEffortSupport,
    #[case] expected: ThinkingLevel,
    #[case] include_thoughts: bool,
) {
    let mapped =
        map_effort_to_level(effort, model, support).expect("every effort here is supported");

    assert_eq!(mapped, level_config(expected, include_thoughts));
}

#[test]
fn an_unsupported_effort_has_no_thinking_level() {
    let error = map_effort_to_level("extreme", Some("gemini-3-flash"), &Supported)
        .expect_err("extreme is not a supported effort");

    assert_eq!(error.status(), Some(400));
}

#[rstest]
#[case::flash_supported("gemini-3-flash-preview", &Supported, true)]
#[case::flash_opted_out("gemini-3-flash-preview", &OptedOut, false)]
#[case::flash_lite("gemini-3-flash-lite", &Supported, true)]
#[case::pro("gemini-3-pro-preview", &Supported, false)]
#[case::older_flash("gemini-2.5-flash", &Supported, false)]
#[case::gemma("gemma-3-flash", &Supported, false)]
fn minimal_thinking_levels_need_a_flash_gemini_3(
    #[case] model: &str,
    #[case] support: &dyn MinimalEffortSupport,
    #[case] expected: bool,
) {
    assert_eq!(supports_minimal_level(model, support), expected);
}

struct ThinkingParamCase {
    generation: GeminiThinkingModel,
    kind: ThinkingType,
    budget_tokens: Option<i64>,
    support: Support,
    settings: ThinkingSettings,
    expected: ThinkingConfig,
}

#[rstest]
#[case::zero_suppresses(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Enabled, budget_tokens: Some(0), support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(false), thinking_budget: None, thinking_level: None } })]
#[case::absent_keeps_thoughts(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Enabled, budget_tokens: None, support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: None } })]
#[case::budget_keeps_thoughts(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Enabled, budget_tokens: Some(2000), support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: None } })]
#[case::default_low_minimal(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Enabled, budget_tokens: None, support: Support::Supported, settings: SETTINGS_LOW, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: Some(ThinkingLevel::Minimal) } })]
#[case::default_low_falls_back(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Enabled, budget_tokens: None, support: Support::OptedOut, settings: SETTINGS_LOW, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: Some(ThinkingLevel::Low) } })]
#[case::default_low_budget(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Enabled, budget_tokens: Some(2000), support: Support::Supported, settings: SETTINGS_LOW, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: Some(ThinkingLevel::Minimal) } })]
#[case::disabled_suppresses(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Disabled, budget_tokens: Some(0), support: Support::Supported, settings: SETTINGS_LOW, expected: ThinkingConfig { include_thoughts: Some(false), thinking_budget: None, thinking_level: None } })]
#[case::adaptive_suppresses(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Adaptive, budget_tokens: None, support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(false), thinking_budget: None, thinking_level: None } })]
#[case::adaptive_ignores_the_default_low(ThinkingParamCase { generation: GeminiThinkingModel::Gemini3, kind: ThinkingType::Adaptive, budget_tokens: Some(5), support: Support::Supported, settings: SETTINGS_LOW, expected: ThinkingConfig { include_thoughts: Some(false), thinking_budget: None, thinking_level: None } })]
#[case::older_zero_budget(ThinkingParamCase { generation: GeminiThinkingModel::Older, kind: ThinkingType::Enabled, budget_tokens: Some(0), support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: None, thinking_budget: Some(0), thinking_level: None } })]
#[case::older_budget(ThinkingParamCase { generation: GeminiThinkingModel::Older, kind: ThinkingType::Enabled, budget_tokens: Some(500), support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: Some(500), thinking_level: None } })]
#[case::older_absent_budget(ThinkingParamCase { generation: GeminiThinkingModel::Older, kind: ThinkingType::Enabled, budget_tokens: None, support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: None } })]
#[case::older_disabled_budget(ThinkingParamCase { generation: GeminiThinkingModel::Older, kind: ThinkingType::Disabled, budget_tokens: Some(500), support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: None, thinking_budget: Some(500), thinking_level: None } })]
#[case::older_disabled_no_budget(ThinkingParamCase { generation: GeminiThinkingModel::Older, kind: ThinkingType::Disabled, budget_tokens: None, support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: None, thinking_budget: None, thinking_level: None } })]
#[case::no_model(ThinkingParamCase { generation: GeminiThinkingModel::None, kind: ThinkingType::Enabled, budget_tokens: None, support: Support::Supported, settings: SETTINGS, expected: ThinkingConfig { include_thoughts: Some(true), thinking_budget: None, thinking_level: None } })]
fn thinking_params_follow_the_model_generation(#[case] case: ThinkingParamCase) {
    let param = ThinkingParam {
        kind: Some(case.kind),
        budget_tokens: case.budget_tokens,
    };

    let mapped = map_thinking_param(
        &param,
        case.generation.model(),
        &case.settings,
        case.support.model_support(),
    );

    assert_eq!(mapped, case.expected);
}

#[derive(Clone, Copy, Debug)]
enum GeminiThinkingModel {
    Gemini3,
    Older,
    None,
}

impl GeminiThinkingModel {
    const fn model(self) -> Option<&'static str> {
        match self {
            Self::Gemini3 => Some("gemini-3-pro-preview"),
            Self::Older => Some("gemini-2.5-flash"),
            Self::None => None,
        }
    }
}

#[test]
fn a_missing_thinking_type_reads_as_disabled() {
    let param = ThinkingParam {
        kind: None,
        budget_tokens: Some(500),
    };

    let mapped = map_thinking_param(
        &param,
        GeminiThinkingModel::Gemini3.model(),
        &SETTINGS,
        &Supported,
    );

    assert_eq!(
        mapped,
        ThinkingConfig {
            include_thoughts: Some(false),
            thinking_budget: None,
            thinking_level: None,
        }
    );
}

#[rstest]
#[case::existing_level(r#"{"thinkingConfig": {"thinkingLevel": "high"}}"#, true)]
#[case::existing_thoughts_only(r#"{"thinkingConfig": {"includeThoughts": true}}"#, false)]
#[case::existing_budget(r#"{"thinkingConfig": {"thinkingBudget": 10}}"#, false)]
#[case::no_thinking_config("{}", false)]
#[case::other_params(r#"{"temperature": 1}"#, false)]
fn an_existing_thinking_level_conflicts(#[case] optional_params: &str, #[case] conflicts: bool) {
    let result = validate_thinking_conflicts(&object(optional_params), "reasoning_effort");

    assert_eq!(result.is_err(), conflicts);
    if let Err(error) = result {
        assert_eq!(error.status(), Some(400));
        assert!(matches!(
            error,
            GeminiThinkingError::ThinkingLevelConflict { .. }
        ));
    }
}

#[rstest]
#[case::known(&["text", "image", "audio", "video"], &[ResponseModality::Text, ResponseModality::Image, ResponseModality::Audio, ResponseModality::Video])]
#[case::unknown(&["text", "modality", "TEXT"], &[ResponseModality::Text, ResponseModality::Unspecified, ResponseModality::Unspecified])]
#[case::empty(&[], &[])]
fn modalities_fall_back_to_unspecified(
    #[case] modalities: &[&str],
    #[case] expected: &[ResponseModality],
) {
    let mapped_input: Vec<String> = modalities.iter().map(|value| (*value).to_owned()).collect();

    assert_eq!(map_response_modalities(&mapped_input), expected);
}

#[test]
fn pcm16_audio_maps_voice_and_language() {
    let audio = AudioParam {
        format: Some("pcm16".to_owned()),
        voice: Some("alloy".to_owned()),
        language_code: Some("en-US".to_owned()),
    };

    let mapped = map_audio_params(&audio).expect("pcm16 is the Gemini TTS format");

    assert_eq!(
        mapped,
        object(
            r#"{
                "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "alloy"}},
                "languageCode": "en-US"
            }"#
        )
    );
}

#[test]
fn absent_audio_fields_are_omitted() {
    let audio = AudioParam {
        format: None,
        voice: Some("alloy".to_owned()),
        language_code: None,
    };

    let mapped = map_audio_params(&audio).expect("an absent format is the pcm16 default");

    assert_eq!(
        mapped,
        object(r#"{"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "alloy"}}}"#)
    );
}

#[test]
fn empty_audio_maps_to_an_empty_speech_config() {
    let audio = AudioParam {
        format: None,
        voice: None,
        language_code: None,
    };

    assert_eq!(
        map_audio_params(&audio).expect("an empty audio object is valid"),
        Object::new()
    );
}

#[rstest]
#[case::mp3("mp3")]
#[case::wav("wav")]
#[case::empty("")]
#[case::uppercase("PCM16")]
fn non_pcm16_audio_is_refused(#[case] format: &str) {
    let audio = AudioParam {
        format: Some(format.to_owned()),
        voice: None,
        language_code: None,
    };

    let error = map_audio_params(&audio).expect_err("Gemini TTS only accepts pcm16");

    assert_eq!(error.status(), None);
    assert!(matches!(
        error,
        GeminiThinkingError::UnsupportedAudioFormat { .. }
    ));
}

#[rstest]
#[case::snake_flag(r#"{"include_server_side_tool_invocations": true}"#, "{}")]
#[case::camel_flag(r#"{"includeServerSideToolInvocations": true}"#, "{}")]
#[case::existing_camel("{}", r#"{"includeServerSideToolInvocations": true}"#)]
#[case::existing_snake("{}", r#"{"include_server_side_tool_invocations": true}"#)]
fn server_tool_flags_are_applied_before_conversion(
    #[case] non_default_params: &str,
    #[case] optional_params: &str,
) {
    let mut mapped_params = object(optional_params);

    apply_server_tool_flags(&object(non_default_params), &mut mapped_params);

    assert_eq!(
        mapped_params.get("include_server_side_tool_invocations"),
        Some(&Json::Bool(true))
    );
}

#[rstest]
#[case::both_false(r#"{"include_server_side_tool_invocations": false}"#, "{}")]
#[case::absent("{}", "{}")]
fn a_false_server_tool_flag_is_not_applied(
    #[case] non_default_params: &str,
    #[case] optional_params: &str,
) {
    let mut mapped_params = object(optional_params);

    apply_server_tool_flags(&object(non_default_params), &mut mapped_params);

    assert!(!mapped_params.contains_key("include_server_side_tool_invocations"));
}

#[test]
fn the_server_tool_flag_keeps_other_params() {
    let mut optional_params = object(r#"{"temperature": 1}"#);

    apply_server_tool_flags(
        &object(r#"{"includeServerSideToolInvocations": true}"#),
        &mut optional_params,
    );

    assert_eq!(
        optional_params.get("temperature"),
        Some(&Json::Number(serde_json::Number::from(1_i64)))
    );
    assert_eq!(optional_params.len(), 2);
}

#[test]
fn the_conflict_message_names_the_parameter() {
    let error = validate_thinking_conflicts(
        &object(r#"{"thinkingConfig": {"thinkingLevel": "low"}}"#),
        "thinking",
    )
    .expect_err("an existing thinking level is a conflict");

    assert!(error.to_string().contains("thinking"));
}
