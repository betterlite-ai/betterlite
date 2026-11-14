#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_error::error::ErrorClass;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;

use super::reasoning::supports_together_reasoning;
use super::{
    LookupFailed, OptionMapping, TogetherAiChatConfig, TogetherCapabilities, TogetherChatError,
    TogetherFlag,
};
use crate::chat::{ChatMessage, Role};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use crate::reasoning_models::DropParams;

#[derive(Default)]
struct Answers {
    checks: BTreeMap<TogetherFlag, bool>,
    failed: bool,
    registry: BTreeMap<TogetherFlag, bool>,
    declared: Option<Vec<String>>,
}

impl TogetherCapabilities for Answers {
    fn check(&self, flag: TogetherFlag, _model: &str) -> Result<bool, LookupFailed> {
        if self.failed {
            return Err(LookupFailed {
                message: String::from("the catalog lookup timed out"),
            });
        }
        Ok(self.checks.get(&flag).copied().unwrap_or(false))
    }

    fn registry_flag(&self, flag: TogetherFlag, _model: &str) -> Option<bool> {
        self.registry.get(&flag).copied()
    }

    fn declared_reasoning_efforts(&self, _model: &str) -> Option<Vec<String>> {
        self.declared.clone()
    }
}

fn config() -> TogetherAiChatConfig {
    TogetherAiChatConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        },
    }
}

fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn message(role: Role, extra: &str) -> ChatMessage {
    let mut body = Object::new();
    body.insert(String::from("role"), serde_json::to_value(role).unwrap());
    body.insert(String::from("content"), Value::String(String::from("hi")));
    body.extend(object(extra));
    serde_json::from_value(Value::Object(body)).unwrap()
}

fn map_existing(
    model: &str,
    requested: &str,
    existing: &str,
    drop_params: DropParams,
    answers: &Answers,
) -> Result<Object, TogetherChatError> {
    let non_default_params = object(requested);
    let mut optional_params = object(existing);
    config().map_openai_params(
        OptionMapping {
            model,
            non_default_params: &non_default_params,
            optional_params: &mut optional_params,
            catalog: &empty_catalog(),
            drop_params,
        },
        answers,
    )?;
    Ok(optional_params)
}

fn map_with(
    model: &str,
    requested: &str,
    drop_params: DropParams,
    answers: &Answers,
) -> Result<Object, TogetherChatError> {
    map_existing(model, requested, "{}", drop_params, answers)
}

fn false_verdict(flag: TogetherFlag) -> Answers {
    Answers {
        registry: BTreeMap::from([(flag, false)]),
        ..Answers::default()
    }
}

fn checked_true(flag: TogetherFlag) -> Answers {
    Answers {
        checks: BTreeMap::from([(flag, true)]),
        ..Answers::default()
    }
}

#[test]
fn assistant_messages_lose_only_the_internal_fields() {
    let mut messages = vec![message(
        Role::Assistant,
        r#"{
            "thinking_blocks": [{"type": "thinking"}],
            "provider_specific_fields": {"cached": true},
            "reasoning_content": "kept",
            "audio": {"id": "a"}
        }"#,
    )];
    TogetherAiChatConfig::strip_internal_fields(&mut messages);
    let expected = IndexMap::from([
        (String::from("reasoning_content"), value(r#""kept""#)),
        (String::from("audio"), value(r#"{"id": "a"}"#)),
    ]);
    assert_eq!(messages[0].passthrough, expected);
}

#[rstest]
#[case::user(Role::User)]
#[case::system(Role::System)]
#[case::developer(Role::Developer)]
#[case::tool(Role::Tool)]
#[case::function(Role::Function)]
fn other_roles_keep_the_internal_fields(#[case] role: Role) {
    let mut messages = vec![message(
        role,
        r#"{"thinking_blocks": [], "provider_specific_fields": {}}"#,
    )];
    TogetherAiChatConfig::strip_internal_fields(&mut messages);
    assert_eq!(messages[0].passthrough.len(), 2);
}

#[test]
fn a_message_without_the_internal_fields_is_unchanged() {
    let original = message(Role::Assistant, r#"{"reasoning_content": "kept"}"#);
    let mut messages = vec![original.clone()];
    TogetherAiChatConfig::strip_internal_fields(&mut messages);
    assert_eq!(messages, vec![original]);
}

#[rstest]
#[case::adjustable_120b("openai/gpt-oss-120b")]
#[case::adjustable_20b("openai/gpt-oss-20b")]
#[case::hybrid_minimax("MiniMaxAI/MiniMax-M3")]
#[case::hybrid_qwen_9b("Qwen/Qwen3.5-9B")]
#[case::hybrid_qwen_plus("Qwen/Qwen3.6-Plus")]
#[case::hybrid_deepseek("deepseek-ai/DeepSeek-V4-Pro")]
#[case::hybrid_kimi("moonshotai/Kimi-K3")]
#[case::hybrid_nvidia("nvidia/nemotron-3-ultra-550b-a55b")]
#[case::hybrid_glm("zai-org/GLM-5.2")]
#[case::high_max_prefix("deepseek-ai/DeepSeek-V4-Pro-plus")]
fn reasoning_models_advertise_reasoning_effort_last(#[case] model: &str) {
    let params = config().supported_params(model, &empty_catalog(), &Answers::default());
    let mut expected = config().base.supported_params(model, &empty_catalog());
    expected.push("reasoning_effort");
    assert_eq!(params, expected);
}

#[rstest]
#[case::unknown("some/model")]
#[case::deepseek_v3("deepseek-ai/DeepSeek-V3")]
#[case::similar_prefix("DeepSeek-V4-Pro-turbo")]
fn other_models_get_the_base_list(#[case] model: &str) {
    let params = config().supported_params(model, &empty_catalog(), &Answers::default());
    assert_eq!(
        params,
        config().base.supported_params(model, &empty_catalog())
    );
}

#[test]
fn a_capability_answer_for_reasoning_appends_reasoning_effort() {
    let answers = checked_true(TogetherFlag::SupportsReasoning);
    let params = config().supported_params("custom/reasoning", &empty_catalog(), &answers);
    assert_eq!(params.last(), Some(&"reasoning_effort"));
}

#[test]
fn a_failed_reasoning_check_is_not_support() {
    let answers = Answers {
        failed: true,
        ..checked_true(TogetherFlag::SupportsReasoning)
    };
    assert!(!supports_together_reasoning("custom/reasoning", &answers));
}

#[test]
fn a_true_tool_verdict_keeps_every_tool_param() {
    let answers = checked_true(TogetherFlag::SupportsFunctionCalling);
    let mapped = map_with(
        "together/qwen",
        r#"{
            "tools": [{"type": "function"}],
            "tool_choice": "auto",
            "function_call": {"name": "x"}
        }"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(mapped.get("tool_choice"), Some(&value(r#""auto""#)));
    assert_eq!(
        mapped.get("function_call"),
        Some(&value(r#"{"name": "x"}"#))
    );
    assert_eq!(
        mapped.get("tools"),
        Some(&value(r#"[{"type": "function"}]"#))
    );
}

#[test]
fn an_unknown_tool_verdict_keeps_the_params() {
    let mapped = map_with(
        "together/qwen",
        r#"{"tools": [], "tool_choice": "auto"}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert_eq!(mapped.get("tools"), Some(&value("[]")));
    assert_eq!(mapped.get("tool_choice"), Some(&value(r#""auto""#)));
}

#[test]
fn a_false_tool_verdict_without_drop_fails() {
    let error = map_with(
        "together/qwen",
        r#"{"tools": [], "tool_choice": "auto", "function_call": {"name": "x"}}"#,
        DropParams::default(),
        &false_verdict(TogetherFlag::SupportsFunctionCalling),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "together_ai does not support parameters: tools, tool_choice, function_call, for model=together/qwen. To drop it from the call, set `litellm.drop_params = True`."
    );
    assert_eq!(error.status_code(), 400);
    assert!(matches!(error.class(), ErrorClass::UnsupportedParams(_)));
}

#[test]
fn the_error_names_only_the_tool_params_that_were_passed() {
    let error = map_with(
        "together/qwen",
        r#"{"tool_choice": "auto"}"#,
        DropParams::default(),
        &false_verdict(TogetherFlag::SupportsFunctionCalling),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("parameters: tool_choice, for model=")
    );
}

#[test]
fn the_drop_argument_removes_the_tool_params() {
    let mapped = map_with(
        "together/qwen",
        r#"{"tools": [], "tool_choice": "auto", "function_call": {"name": "x"}}"#,
        DropParams {
            request: true,
            global: false,
        },
        &false_verdict(TogetherFlag::SupportsFunctionCalling),
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn the_global_drop_flag_removes_the_tool_params() {
    let mapped = map_with(
        "together/qwen",
        r#"{"tools": []}"#,
        DropParams {
            request: false,
            global: true,
        },
        &false_verdict(TogetherFlag::SupportsFunctionCalling),
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn a_failed_check_still_consults_the_registry_flag() {
    let answers = Answers {
        failed: true,
        ..false_verdict(TogetherFlag::SupportsFunctionCalling)
    };
    let error = map_with(
        "together/qwen",
        r#"{"tools": []}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap_err();
    assert!(error.to_string().contains("parameters: tools, for model="));
}

#[test]
fn a_stored_true_flag_is_not_a_verdict() {
    let answers = Answers {
        registry: BTreeMap::from([(TogetherFlag::SupportsFunctionCalling, true)]),
        ..Answers::default()
    };
    let mapped = map_with(
        "together/qwen",
        r#"{"tools": []}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(mapped.get("tools"), Some(&value("[]")));
}

#[test]
fn a_true_response_schema_verdict_keeps_response_format() {
    let answers = checked_true(TogetherFlag::SupportsResponseSchema);
    let mapped = map_with(
        "together/qwen",
        r#"{"response_format": {"type": "json_object"}}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(
        mapped.get("response_format"),
        Some(&value(r#"{"type": "json_object"}"#))
    );
}

#[test]
fn an_unknown_response_schema_verdict_keeps_response_format() {
    let mapped = map_with(
        "together/qwen",
        r#"{"response_format": {"type": "json_object"}}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert!(mapped.contains_key("response_format"));
}

#[test]
fn a_false_response_schema_verdict_drops_with_drop_params() {
    let mapped = map_with(
        "together/qwen",
        r#"{"response_format": {"type": "json_object"}}"#,
        DropParams {
            request: true,
            global: false,
        },
        &false_verdict(TogetherFlag::SupportsResponseSchema),
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn the_global_drop_flag_drops_response_format() {
    let mapped = map_with(
        "together/qwen",
        r#"{"response_format": {"type": "json_object"}}"#,
        DropParams {
            request: false,
            global: true,
        },
        &false_verdict(TogetherFlag::SupportsResponseSchema),
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn a_false_response_schema_verdict_fails_without_drop_params() {
    let error = map_with(
        "together/qwen",
        r#"{"response_format": {"type": "json_object"}, "temperature": 0.5}"#,
        DropParams::default(),
        &false_verdict(TogetherFlag::SupportsResponseSchema),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("parameters: response_format, for model=")
    );
    assert_eq!(
        map_with(
            "together/qwen",
            r#"{"temperature": 0.5}"#,
            DropParams::default(),
            &false_verdict(TogetherFlag::SupportsResponseSchema),
        )
        .unwrap()
        .get("temperature"),
        Some(&value("0.5"))
    );
}

#[rstest]
#[case::adjustable_minimal("openai/gpt-oss-20b", "minimal", "low")]
#[case::adjustable_xhigh("openai/gpt-oss-20b", "xhigh", "high")]
#[case::adjustable_max("openai/gpt-oss-20b", "max", "high")]
#[case::adjustable_low("openai/gpt-oss-20b", "low", "low")]
#[case::adjustable_none("openai/gpt-oss-20b", "none", "low")]
#[case::adjustable_unknown("openai/gpt-oss-20b", "custom", "custom")]
#[case::deepseek_minimal("deepseek-ai/DeepSeek-V4-Pro-turbo", "minimal", "high")]
#[case::deepseek_low("deepseek-ai/DeepSeek-V4-Pro-turbo", "low", "high")]
#[case::deepseek_medium("deepseek-ai/DeepSeek-V4-Pro-turbo", "medium", "high")]
#[case::deepseek_xhigh("deepseek-ai/DeepSeek-V4-Pro-turbo", "xhigh", "max")]
#[case::deepseek_high("deepseek-ai/DeepSeek-V4-Pro-turbo", "high", "high")]
#[case::deepseek_unknown("deepseek-ai/DeepSeek-V4-Pro-turbo", "custom", "custom")]
fn reasoning_efforts_reach_the_pinned_payload(
    #[case] model: &str,
    #[case] requested: &str,
    #[case] expected: &str,
) {
    let mapped = map_with(
        model,
        &format!(r#"{{"reasoning_effort": "{requested}"}}"#),
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert_eq!(
        mapped,
        object(&format!(r#"{{"reasoning_effort": "{expected}"}}"#))
    );
}

#[test]
fn the_default_effort_leaves_no_keys() {
    let mapped = map_with(
        "openai/gpt-oss-120b",
        r#"{"reasoning_effort": "default"}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn a_none_effort_disables_reasoning_on_other_models() {
    let mapped = map_with(
        "MiniMaxAI/MiniMax-M3",
        r#"{"reasoning_effort": "none"}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning": {"enabled": false}}"#));
}

#[test]
fn a_declared_effort_is_kept_unchanged() {
    let answers = Answers {
        declared: Some(vec![String::from("minimal"), String::from("high")]),
        ..Answers::default()
    };
    let mapped = map_with(
        "MiniMaxAI/MiniMax-M3",
        r#"{"reasoning_effort": "minimal"}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning_effort": "minimal"}"#));
}

#[test]
fn an_undeclared_effort_goes_through_the_translation() {
    let answers = Answers {
        declared: Some(vec![String::from("high")]),
        ..Answers::default()
    };
    let mapped = map_with(
        "MiniMaxAI/MiniMax-M3",
        r#"{"reasoning_effort": "xhigh"}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning_effort": "high"}"#));
}

#[test]
fn a_declared_deepseek_effort_skips_the_high_max_table() {
    let answers = Answers {
        declared: Some(vec![String::from("medium")]),
        ..Answers::default()
    };
    let mapped = map_with(
        "deepseek-ai/DeepSeek-V4-Pro",
        r#"{"reasoning_effort": "medium"}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning_effort": "medium"}"#));
}

#[test]
fn a_caller_set_key_wins_over_the_payload() {
    let mapped = map_existing(
        "MiniMaxAI/MiniMax-M3",
        r#"{"reasoning_effort": "none"}"#,
        r#"{"reasoning": {"enabled": true}}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning": {"enabled": true}}"#));
}

#[test]
fn a_non_string_effort_is_left_untouched() {
    let mapped = map_with(
        "openai/gpt-oss-20b",
        r#"{"reasoning_effort": 5}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning_effort": 5}"#));
}

#[test]
fn the_flag_names_match_litellm() {
    assert_eq!(
        TogetherFlag::SupportsFunctionCalling.name(),
        "supports_function_calling"
    );
    assert_eq!(
        TogetherFlag::SupportsResponseSchema.name(),
        "supports_response_schema"
    );
    assert_eq!(TogetherFlag::SupportsReasoning.name(), "supports_reasoning");
}

#[test]
fn a_plain_model_never_reaches_the_payload_step() {
    let mapped = map_with(
        "some/model",
        r#"{"reasoning_effort": "high"}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn a_capability_reasoning_model_reaches_the_payload_step() {
    let answers = checked_true(TogetherFlag::SupportsReasoning);
    let mapped = map_with(
        "custom/reasoning",
        r#"{"reasoning_effort": "minimal"}"#,
        DropParams::default(),
        &answers,
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"reasoning_effort": "low"}"#));
}

#[test]
fn unsupported_and_null_params_are_filtered_before_the_overrides() {
    let mapped = map_with(
        "some/model",
        r#"{"unknown_param": true, "temperature": 0.2, "stream": true}"#,
        DropParams::default(),
        &Answers::default(),
    )
    .unwrap();
    assert!(!mapped.contains_key("unknown_param"));
    assert_eq!(mapped.get("temperature"), Some(&value("0.2")));
    assert_eq!(mapped.get("stream"), Some(&Value::Bool(true)));
}
