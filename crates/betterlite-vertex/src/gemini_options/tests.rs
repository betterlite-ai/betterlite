#![cfg(test)]

use std::fs;
use std::sync::Arc;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::de::DeserializeOwned;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::assembly::{OptionsContext, map_options};
use super::defaults::{GenerationDefaults, OptionSettings};
use super::error::GeminiOptionsError;
use super::params::{
    EU_REGIONS, ReasoningSupport, US_REGIONS, map_special_auth_params, supported_openai_params,
};
use super::projection::{SchemaSource, project_schema_object};
use crate::gemini_thinking::settings::{MinimalEffortSupport, ThinkingBudgets, ThinkingSettings};

fn parse<T>(source: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    parse(source)
}

fn options(source: &str) -> Object {
    parse(source)
}

fn thinking_settings() -> ThinkingSettings {
    ThinkingSettings {
        budgets: ThinkingBudgets {
            minimal: 128,
            minimal_flash: 1,
            minimal_pro: 128,
            minimal_flash_lite: 512,
            low: 1024,
            medium: 2048,
            high: 4096,
            disable: 0,
        },
        default_thinking_level_low: false,
    }
}

struct NoMinimalEffort;

impl MinimalEffortSupport for NoMinimalEffort {
    fn disables_minimal(&self, _model: &str) -> bool {
        false
    }
}

fn map_with(
    model: &str,
    non_default: &Object,
    optional: &mut Object,
    settings: &OptionSettings,
) -> Result<usize, GeminiOptionsError> {
    let thinking = thinking_settings();
    let schema = SchemaSettings::default();
    let support = NoMinimalEffort;
    let context = OptionsContext {
        model,
        non_default,
        settings,
        schema: &schema,
        thinking: &thinking,
        support: &support,
    };
    map_options(non_default, optional, &context)
}

fn mapped(model: &str, source: &str) -> (Json, usize) {
    let non_default = options(source);
    let mut optional = Object::new();
    let warnings = map_with(
        model,
        &non_default,
        &mut optional,
        &OptionSettings::default(),
    )
    .unwrap();
    (Json::Object(optional), warnings)
}

fn failure(model: &str, source: &str) -> GeminiOptionsError {
    let non_default = options(source);
    let mut optional = Object::new();
    map_with(
        model,
        &non_default,
        &mut optional,
        &OptionSettings::default(),
    )
    .expect_err("the value is refused")
}

#[test]
fn sampling_values_are_kept_and_warn_once() {
    let (result, warnings) = mapped(
        "gemini-3-pro-preview",
        r#"{"temperature": 0.5, "top_p": 0.9, "top_k": 5}"#,
    );

    assert_eq!(
        result,
        json(r#"{"temperature": 0.5, "top_p": 0.9, "top_k": 5}"#)
    );
    assert_eq!(warnings, 1);
}

#[test]
fn sampling_warns_each_request_and_logs_info_only_below_one() {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    with_default(subscriber, || {
        mapped("gemini-3-pro-preview", r#"{"temperature": 0.2}"#);
        let after_below_one = fs::read_to_string(log.path()).unwrap();
        assert_eq!(
            after_below_one.matches("INFO").count(),
            1,
            "{after_below_one}"
        );
        mapped("gemini-3-pro-preview", r#"{"top_p": 0.9}"#);
        mapped("gemini-3-pro-preview", r#"{"temperature": 1.0}"#);
    });
    let logged = fs::read_to_string(log.path()).unwrap();
    assert_eq!(logged.matches("WARN").count(), 3, "{logged}");
    assert_eq!(logged.matches("INFO").count(), 1, "{logged}");
    assert!(logged.contains("planned for removal"), "{logged}");
}

#[rstest]
#[case::explicit_zero_on_an_older_model(
    "gemini-2.5-pro",
    r#"{"temperature": 0}"#,
    r#"{"temperature": 0}"#
)]
#[case::explicit_zero_on_gemini_three(
    "gemini-3-pro-preview",
    r#"{"temperature": 0}"#,
    r#"{"temperature": 0}"#
)]
#[case::absent_on_gemini_three("gemini-3-pro-preview", "{}", r#"{"temperature": 1.0}"#)]
#[case::absent_on_an_older_model("gemini-2.5-pro", "{}", "{}")]
fn an_explicit_zero_survives_and_gemini_three_defaults_to_one(
    #[case] model: &str,
    #[case] request: &str,
    #[case] expected: &str,
) {
    let (result, _) = mapped(model, request);

    assert_eq!(result, json(expected));
}

#[rstest]
#[case::older_model(
    "gemini-2.5-pro",
    r#"{"frequency_penalty": 0.1, "presence_penalty": 0.2}"#
)]
#[case::gemini_three("gemini-3-pro-preview", r#"{"temperature": 1.0}"#)]
#[case::the_preview_model("gemini-2.5-pro-preview-06-05", "{}")]
#[case::a_qualified_preview_model(
    "vertex_ai/gemini-2.5-pro-preview-06-05",
    r#"{"frequency_penalty": 0.1, "presence_penalty": 0.2}"#
)]
fn penalties_follow_the_model(#[case] model: &str, #[case] expected: &str) {
    let (result, _) = mapped(
        model,
        r#"{"frequency_penalty": 0.1, "presence_penalty": 0.2}"#,
    );

    assert_eq!(result, json(expected));
}

#[rstest]
#[case::tokens_then_completion(
    r#"{"max_tokens": 100, "max_completion_tokens": 200}"#,
    r#"{"max_output_tokens": 200}"#
)]
#[case::completion_then_tokens(
    r#"{"max_completion_tokens": 200, "max_tokens": 100}"#,
    r#"{"max_output_tokens": 100}"#
)]
fn the_last_token_limit_wins(#[case] request: &str, #[case] expected: &str) {
    let (result, _) = mapped("gemini-2.5-pro", request);

    assert_eq!(result, json(expected));
}

#[test]
fn audio_adds_the_audio_modality() {
    let (result, warnings) = mapped(
        "gemini-2.5-pro",
        r#"{"audio": {"format": "pcm16", "voice": "en-US-Studio-O"}}"#,
    );

    assert_eq!(
        result,
        json(
            r#"{
                "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "en-US-Studio-O"}}},
                "responseModalities": ["AUDIO"]
            }"#
        )
    );
    assert_eq!(warnings, 0);
}

#[test]
fn audio_appends_audio_to_the_requested_modalities() {
    let (result, _) = mapped(
        "gemini-2.5-pro",
        r#"{"modalities": ["text"], "audio": {"format": "pcm16"}}"#,
    );

    assert_eq!(
        result,
        json(r#"{"responseModalities": ["TEXT", "AUDIO"], "speechConfig": {}}"#)
    );
}

#[test]
fn an_audio_modality_is_not_duplicated() {
    let (result, _) = mapped(
        "gemini-2.5-pro",
        r#"{"modalities": ["audio", "text"], "audio": {}}"#,
    );

    assert_eq!(
        result,
        json(r#"{"responseModalities": ["AUDIO", "TEXT"], "speechConfig": {}}"#)
    );
}

#[rstest]
#[case::two_tools_drop(r#"{"tools": [{"function": {"name": "a"}}, {"function": {"name": "b"}}], "parallel_tool_calls": false}"#, false)]
#[case::one_tool_keeps(
    r#"{"tools": [{"function": {"name": "a"}}], "parallel_tool_calls": false}"#,
    true
)]
#[case::two_tools_and_true_keeps(r#"{"tools": [{"function": {"name": "a"}}, {"function": {"name": "b"}}], "parallel_tool_calls": true}"#, true)]
fn parallel_tool_calls_false_is_dropped_only_with_several_tools(
    #[case] request: &str,
    #[case] kept: bool,
) {
    let (result, _) = mapped("gemini-2.5-pro", request);

    let Json::Object(fields) = result else {
        panic!("the mapping answers an object");
    };
    assert_eq!(fields.contains_key("parallel_tool_calls"), kept);
}

#[test]
fn the_server_flag_applies_before_the_tools_are_mapped() {
    let flagged = r#"{
        "tools": [{"function": {"name": "lookup"}}, {"googleSearch": {}}],
        "includeServerSideToolInvocations": true
    }"#;
    let (result, warnings) = mapped("gemini-2.5-pro", flagged);

    assert_eq!(
        result,
        json(
            r#"{
                "include_server_side_tool_invocations": true,
                "tools": [{"function_declarations": [{"name": "lookup"}]}, {"googleSearch": {}}]
            }"#
        )
    );
    assert_eq!(warnings, 0);
}

#[test]
fn a_search_tool_mixed_with_functions_is_dropped_without_the_flag() {
    let (result, warnings) = mapped(
        "gemini-2.5-pro",
        r#"{"tools": [{"function": {"name": "lookup"}}], "web_search_options": {}}"#,
    );

    assert_eq!(
        result,
        json(r#"{"tools": [{"function_declarations": [{"name": "lookup"}]}]}"#)
    );
    assert_eq!(warnings, 1);
}

#[test]
fn the_module_safety_override_replaces_the_request_value() {
    let request = options(r#"{"safety_settings": [{"category": "REQUEST"}]}"#);
    let settings = OptionSettings {
        generation: GenerationDefaults::default(),
        safety_settings: Some(json(r#"[{"category": "CONFIGURED"}]"#)),
    };
    let mut optional = Object::new();
    map_with("gemini-2.5-pro", &request, &mut optional, &settings).unwrap();

    assert_eq!(
        Json::Object(optional),
        json(r#"{"safety_settings": [{"category": "CONFIGURED"}]}"#)
    );
}

#[test]
fn an_unset_safety_override_keeps_the_request_value() {
    let request = options(r#"{"temperature": 0.5}"#);
    let mut optional = options(r#"{"safety_settings": [{"category": "REQUEST"}]}"#);
    map_with(
        "gemini-2.5-pro",
        &request,
        &mut optional,
        &OptionSettings::default(),
    )
    .unwrap();

    assert_eq!(
        Json::Object(optional),
        json(r#"{"safety_settings": [{"category": "REQUEST"}], "temperature": 0.5}"#)
    );
}

#[test]
fn stream_false_is_omitted() {
    let (result, _) = mapped("gemini-2.5-pro", r#"{"stream": false}"#);
    assert_eq!(result, json("{}"));

    let (enabled, _) = mapped("gemini-2.5-pro", r#"{"stream": true}"#);
    assert_eq!(enabled, json(r#"{"stream": true}"#));
}

#[rstest]
#[case::n(r#"{"n": 2}"#, r#"{"candidate_count": 2}"#)]
#[case::seed(r#"{"seed": 7}"#, r#"{"seed": 7}"#)]
#[case::logprobs(r#"{"logprobs": true}"#, r#"{"responseLogprobs": true}"#)]
#[case::top_logprobs(r#"{"top_logprobs": 3}"#, r#"{"logprobs": 3}"#)]
#[case::stop_string(r#"{"stop": "\n\n"}"#, r#"{"stop_sequences": ["\n\n"]}"#)]
#[case::stop_list(r#"{"stop": ["a", "b"]}"#, r#"{"stop_sequences": ["a", "b"]}"#)]
#[case::service_tier_auto(r#"{"service_tier": "auto"}"#, r#"{"service_tier": "priority"}"#)]
#[case::service_tier_other(r#"{"service_tier": "Standard"}"#, r#"{"service_tier": "standard"}"#)]
#[case::modalities(
    r#"{"modalities": ["text", "video", "unknown"]}"#,
    r#"{"responseModalities": ["TEXT", "VIDEO", "MODALITY_UNSPECIFIED"]}"#
)]
fn scalar_parameters_map_to_their_gemini_names(#[case] request: &str, #[case] expected: &str) {
    let (result, _) = mapped("gemini-2.5-pro", request);

    assert_eq!(result, json(expected));
}

#[test]
fn tool_choice_maps_a_string_and_a_function_object() {
    let (named, _) = mapped("gemini-2.5-pro", r#"{"tool_choice": "required"}"#);
    assert_eq!(
        named,
        json(r#"{"tool_choice": {"functionCallingConfig": {"mode": "ANY"}}}"#)
    );

    let (chosen, _) = mapped(
        "gemini-2.5-pro",
        r#"{"tool_choice": {"function": {"name": "lookup"}}}"#,
    );
    assert_eq!(
        chosen,
        json(
            r#"{"tool_choice": {"functionCallingConfig": {"mode": "ANY", "allowed_function_names": ["lookup"]}}}"#
        )
    );
}

#[test]
fn web_search_options_become_a_google_search_tool() {
    let (result, _) = mapped(
        "gemini-2.5-pro",
        r#"{"web_search_options": {"search_context_size": "high"}}"#,
    );

    assert_eq!(result, json(r#"{"tools": [{"googleSearch": {}}]}"#));
}

#[test]
fn tools_and_functions_append_in_request_order() {
    let (result, _) = mapped(
        "gemini-2.5-pro",
        r#"{"tools": [{"function": {"name": "first"}}], "functions": [{"name": "second"}]}"#,
    );

    assert_eq!(
        result,
        json(
            r#"{"tools": [
                {"function_declarations": [{"name": "first"}]},
                {"function_declarations": [{"name": "second"}]}
            ]}"#
        )
    );
}

#[test]
fn reasoning_effort_maps_to_a_level_or_a_budget() {
    let (budget, _) = mapped("gemini-2.5-pro", r#"{"reasoning_effort": "high"}"#);
    assert_eq!(
        budget,
        json(r#"{"thinkingConfig": {"includeThoughts": true, "thinkingBudget": 4096}}"#)
    );

    let (level, _) = mapped("gemini-3-pro-preview", r#"{"reasoning_effort": "high"}"#);
    assert_eq!(
        level,
        json(
            r#"{"thinkingConfig": {"includeThoughts": true, "thinkingLevel": "high"}, "temperature": 1.0}"#
        )
    );
}

#[test]
fn a_thinking_budget_conflicting_with_a_level_is_refused() {
    let request = options(r#"{"thinking": {"type": "enabled", "budget_tokens": 500}}"#);
    let mut optional = options(r#"{"thinkingConfig": {"thinkingLevel": "medium"}}"#);
    let error = map_with(
        "gemini-3-pro-preview",
        &request,
        &mut optional,
        &OptionSettings::default(),
    )
    .expect_err("a level is already set");

    assert_eq!(error.status(), Some(400));
}

#[rstest]
#[case::temperature(r#"{"temperature": "warm"}"#, "temperature")]
#[case::stream(r#"{"stream": "yes"}"#, "stream")]
#[case::stop(r#"{"stop": 5}"#, "stop")]
#[case::tools(r#"{"tools": "lookup"}"#, "tools")]
#[case::parallel(r#"{"parallel_tool_calls": 1}"#, "parallel_tool_calls")]
#[case::audio(r#"{"audio": "voice"}"#, "audio")]
#[case::modalities(r#"{"modalities": "text"}"#, "modalities")]
fn a_wrong_type_names_its_field(#[case] request: &str, #[case] param: &str) {
    let error = failure("gemini-2.5-pro", request);

    assert!(error.to_string().contains(param), "{error}");
    assert_eq!(error.status(), Some(400));
}

#[test]
fn an_unsupported_tool_choice_is_a_bad_request() {
    let error = failure("gemini-2.5-pro", r#"{"tool_choice": "sometimes"}"#);

    assert_eq!(error.status(), Some(400));
}

#[test]
fn an_unsupported_audio_format_is_a_server_error() {
    let error = failure(
        "gemini-2.5-pro",
        r#"{"audio": {"format": "mp3", "voice": "alloy"}}"#,
    );

    assert_eq!(error.status(), Some(500));
}

#[test]
fn get_config_returns_only_the_configured_values() {
    assert_eq!(
        GenerationDefaults::default().get_config().unwrap(),
        Object::new()
    );
    let configured = GenerationDefaults {
        temperature: Some(0.0_f64),
        max_output_tokens: Some(256),
        top_p: Some(0.95_f64),
        top_k: Some(40),
        response_mime_type: Some("application/json".to_owned()),
        candidate_count: Some(2),
        stop_sequences: Some(vec!["END".to_owned()]),
        frequency_penalty: Some(0.1_f64),
        presence_penalty: Some(0.2_f64),
        seed: Some(7),
    };
    assert_eq!(
        Json::Object(configured.get_config().unwrap()),
        json(
            r#"{
                "temperature": 0.0,
                "max_output_tokens": 256,
                "top_p": 0.95,
                "top_k": 40,
                "response_mime_type": "application/json",
                "candidate_count": 2,
                "stop_sequences": ["END"],
                "frequency_penalty": 0.1,
                "presence_penalty": 0.2,
                "seed": 7
            }"#
        )
    );
}

#[test]
fn a_configured_default_never_overrides_the_request() {
    let mut supplied = options(r#"{"temperature": 0.2}"#);
    GenerationDefaults {
        temperature: Some(0.9_f64),
        max_output_tokens: None,
        top_p: None,
        top_k: None,
        response_mime_type: None,
        candidate_count: None,
        stop_sequences: None,
        frequency_penalty: None,
        presence_penalty: None,
        seed: Some(7),
    }
    .merge_into(&mut supplied)
    .unwrap();

    assert_eq!(
        Json::Object(supplied),
        json(r#"{"temperature": 0.2, "seed": 7}"#)
    );
}

#[test]
fn a_non_finite_default_names_itself() {
    let error = GenerationDefaults {
        temperature: Some(f64::NAN),
        max_output_tokens: None,
        top_p: None,
        top_k: None,
        response_mime_type: None,
        candidate_count: None,
        stop_sequences: None,
        frequency_penalty: None,
        presence_penalty: None,
        seed: None,
    }
    .get_config()
    .expect_err("NaN has no JSON number");

    assert!(error.to_string().contains("temperature"), "{error}");
}

#[test]
fn auth_aliases_map_the_request_values() {
    let request = options(r#"{"project": "my-project", "region_name": "us-central1"}"#);
    let optional = options(r#"{"vertex_project": "old"}"#);
    let mapped = map_special_auth_params(&request, optional).unwrap();

    assert_eq!(
        Json::Object(mapped),
        json(r#"{"vertex_project": "my-project", "vertex_location": "us-central1"}"#)
    );
}

#[test]
fn the_region_lists_are_the_source_data() {
    assert_eq!(
        EU_REGIONS,
        [
            "europe-central2",
            "europe-north1",
            "europe-southwest1",
            "europe-west1",
            "europe-west2",
            "europe-west3",
            "europe-west4",
            "europe-west6",
            "europe-west8",
            "europe-west9",
        ]
    );
    assert_eq!(
        US_REGIONS,
        [
            "us-central1",
            "us-east1",
            "us-east4",
            "us-east5",
            "us-south1",
            "us-west1",
            "us-west4",
            "us-west5",
        ]
    );
}

#[test]
fn a_python_model_wraps_its_schema_with_strict() {
    let schema = options(r#"{"type": "object", "$defs": {"City": {"type": "string"}}}"#);
    let projected = project_schema_object(SchemaSource::PythonModel {
        name: "Answer".to_owned(),
        schema,
    });

    assert_eq!(
        Json::Object(projected),
        json(
            r#"{
                "type": "json_schema",
                "json_schema": {
                    "schema": {"type": "object", "$defs": {"City": {"type": "string"}}},
                    "name": "Answer",
                    "strict": true
                }
            }"#
        )
    );
}

#[test]
fn a_dictionary_schema_passes_through() {
    let format = options(r#"{"type": "json_object"}"#);
    let projected = project_schema_object(SchemaSource::Dictionary(format));

    assert_eq!(Json::Object(projected), json(r#"{"type": "json_object"}"#));
}

struct Catalog(bool);

impl ReasoningSupport for Catalog {
    fn supports_reasoning(&self, _model: &str) -> bool {
        self.0
    }
}

// LiteLLM's get_supported_openai_params lists these names in this order before the model-dependent groups
const BASE_PARAMS: [&str; 22] = [
    "temperature",
    "top_p",
    "top_k",
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "tools",
    "functions",
    "tool_choice",
    "response_format",
    "n",
    "stop",
    "extra_headers",
    "seed",
    "logprobs",
    "top_logprobs",
    "modalities",
    "audio",
    "parallel_tool_calls",
    "web_search_options",
    "include_server_side_tool_invocations",
    "service_tier",
];

#[rstest]
#[case::older_model("gemini-2.5-pro", false, &["frequency_penalty", "presence_penalty"])]
#[case::reasoning_model(
    "gemini-2.5-pro",
    true,
    &["frequency_penalty", "presence_penalty", "reasoning_effort", "thinking"]
)]
#[case::gemini_three("gemini-3-pro-preview", false, &["reasoning_effort", "thinking"])]
#[case::the_preview_model("gemini-2.5-pro-preview-06-05", false, &[])]
fn supported_parameters_follow_the_model(
    #[case] model: &str,
    #[case] reasoning: bool,
    #[case] extras: &[&str],
) {
    let mut expected = BASE_PARAMS.to_vec();
    expected.extend_from_slice(extras);

    assert_eq!(
        supported_openai_params(model, &Catalog(reasoning)),
        expected
    );
}

#[test]
fn an_unknown_reasoning_effort_keeps_its_status() {
    let error = failure("gemini-2.5-pro", r#"{"reasoning_effort": "extreme"}"#);

    assert_eq!(error.status(), Some(400));
    assert!(error.to_string().contains("extreme"), "{error}");
}

#[test]
fn a_null_value_is_absent() {
    let (result, warnings) = mapped("gemini-2.5-pro", r#"{"temperature": null, "seed": 3}"#);

    assert_eq!(result, json(r#"{"seed": 3}"#));
    assert_eq!(warnings, 0);
}

#[test]
fn a_response_format_becomes_a_mime_type() {
    let (result, _) = mapped(
        "gemini-2.5-pro",
        r#"{"response_format": {"type": "json_object"}}"#,
    );

    assert_eq!(
        result,
        json(r#"{"response_mime_type": "application/json"}"#)
    );
}

#[test]
fn a_maps_tool_adds_its_retrieval_config() {
    let (result, _) = mapped(
        "gemini-2.5-pro",
        r#"{"tools": [{"googleMaps": {"latitude": 1.5, "longitude": 2.5, "languageCode": "en"}}]}"#,
    );

    assert_eq!(
        result,
        json(
            r#"{
                "tools": [{"googleMaps": {}}],
                "toolConfig": {"retrievalConfig": {"latLng": {"latitude": 1.5, "longitude": 2.5}, "languageCode": "en"}}
            }"#
        )
    );
}

// LiteLLM adds retrievalConfig to the caller's toolConfig instead of replacing it
#[test]
fn a_maps_tool_merges_into_an_existing_tool_config() {
    let request = options(
        r#"{"tools": [{"googleMaps": {"latitude": 1.5, "longitude": 2.5, "languageCode": "en"}}]}"#,
    );
    let mut optional = options(r#"{"toolConfig": {"functionCallingConfig": {"mode": "AUTO"}}}"#);
    map_with(
        "gemini-2.5-pro",
        &request,
        &mut optional,
        &OptionSettings::default(),
    )
    .unwrap();

    assert_eq!(
        Json::Object(optional),
        json(
            r#"{
                "toolConfig": {
                    "functionCallingConfig": {"mode": "AUTO"},
                    "retrievalConfig": {"latLng": {"latitude": 1.5, "longitude": 2.5}, "languageCode": "en"}
                },
                "tools": [{"googleMaps": {}}]
            }"#
        )
    );
}
