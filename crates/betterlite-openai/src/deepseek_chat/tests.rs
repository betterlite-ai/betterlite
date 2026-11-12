#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::{Mutex, PoisonError};

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Value;
use tracing::subscriber::with_default;

use super::cost::GenericPricing;
use super::history::{fill_reasoning_content, thinking_active};
use super::identity::complete_url;
use super::messages::forward_or_collapse;
use super::options::{map_openai_params, supported_openai_params};
use super::request::{RequestInput, transform_request};
use super::tools::drop_unsupported_tools;
use crate::chat::ChatMessage;
use crate::chat_content::Mode;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("a JSON value")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).expect("prompt messages")
}

fn chats(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).expect("chat messages")
}

fn transformed_messages(model: &str, messages: &[ChatMessage], supports_vision: bool) -> Value {
    let empty = Object::new();
    let input = RequestInput {
        model,
        messages,
        optional_params: &empty,
        supports_vision,
        supports_reasoning: false,
    };
    let body = transform_request(&input, Mode::Sync).expect("the parent transform");
    rendered(&body)["messages"].clone()
}

fn rendered(mapped: &Object) -> Value {
    serde_json::to_value(mapped).expect("the mapped params")
}

fn openai_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn mapped(requested: &str, existing: &str) -> Value {
    let mut optional = object(existing);
    map_openai_params(
        "deepseek-reasoner",
        &object(requested),
        &mut optional,
        &openai_config(),
        &empty_catalog(),
    );
    rendered(&optional)
}

fn prepared(text: &str, supports_vision: bool) -> Value {
    serde_json::to_value(forward_or_collapse(&messages(text), supports_vision))
        .expect("the prepared messages")
}

fn repaired(text: &str) -> Value {
    serde_json::to_value(fill_reasoning_content(&messages(text))).expect("the repaired messages")
}

fn dropped(text: &str) -> (Value, Vec<String>) {
    let result = drop_unsupported_tools(&object(text));
    (rendered(&result.params), result.dropped_types.clone())
}

fn capture_logs<Action>(action: Action) -> String
where
    Action: FnOnce(),
{
    let log = tempfile::NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().expect("a reopened log"))
        .finish();
    with_default(subscriber, action);
    fs::read_to_string(log.path()).expect("the log")
}

fn occurrences(log: &str, fragment: &str) -> usize {
    log.matches(fragment).count()
}

#[test]
fn supported_params_extend_the_base_list_with_thinking_and_effort() {
    let params = supported_openai_params("deepseek-reasoner", &openai_config(), &empty_catalog());
    assert!(params.contains(&"temperature"));
    assert!(params.ends_with(&["thinking", "reasoning_effort"]));
}

#[test]
fn mapping_copies_supported_params_and_drops_unknown_ones() {
    assert_eq!(
        mapped(r#"{"temperature":0.5,"unknown":1}"#, "{}"),
        value(r#"{"temperature":0.5}"#)
    );
}

#[test]
fn an_enabled_thinking_object_wins_and_loses_budget_tokens() {
    assert_eq!(
        mapped(
            r#"{"thinking":{"type":"enabled","budget_tokens":1024}}"#,
            "{}"
        ),
        value(r#"{"thinking":{"type":"enabled"}}"#)
    );
}

#[test]
fn a_disabled_thinking_object_is_kept() {
    assert_eq!(
        mapped(r#"{"thinking":{"type":"disabled"}}"#, "{}"),
        value(r#"{"thinking":{"type":"disabled"}}"#)
    );
}

#[test]
fn an_enabled_thinking_object_wins_over_a_none_effort() {
    assert_eq!(
        mapped(
            r#"{"thinking":{"type":"enabled"},"reasoning_effort":"none"}"#,
            "{}"
        ),
        value(r#"{"thinking":{"type":"enabled"}}"#)
    );
}

#[rstest]
#[case::none("none", "disabled")]
#[case::low("low", "enabled")]
#[case::high("high", "enabled")]
#[case::empty("", "enabled")]
fn a_non_null_effort_maps_to_thinking(#[case] effort: &str, #[case] kind: &str) {
    let requested = format!(r#"{{"reasoning_effort":"{effort}"}}"#);
    let expected = format!(r#"{{"thinking":{{"type":"{kind}"}}}}"#);
    assert_eq!(mapped(&requested, "{}"), value(&expected));
}

#[test]
fn a_null_effort_adds_no_thinking() {
    assert_eq!(mapped(r#"{"reasoning_effort":null}"#, "{}"), value("{}"));
}

#[test]
fn a_malformed_thinking_object_falls_back_to_the_effort() {
    assert_eq!(
        mapped(
            r#"{"thinking":{"type":"auto"},"reasoning_effort":"high"}"#,
            "{}"
        ),
        value(r#"{"thinking":{"type":"enabled"}}"#)
    );
}

#[test]
fn a_malformed_thinking_object_without_an_effort_is_dropped() {
    assert_eq!(mapped(r#"{"thinking":{"type":"auto"}}"#, "{}"), value("{}"));
}

#[test]
fn a_thinking_array_is_not_a_thinking_object() {
    assert_eq!(
        mapped(
            r#"{"thinking":["enabled"],"reasoning_effort":"none"}"#,
            "{}"
        ),
        value(r#"{"thinking":{"type":"disabled"}}"#)
    );
}

#[test]
fn thinking_and_effort_already_in_the_optional_params_are_rebuilt() {
    assert_eq!(
        mapped(
            "{}",
            r#"{"thinking":{"type":"enabled","budget_tokens":8},"reasoning_effort":"none"}"#
        ),
        value(r#"{"thinking":{"type":"enabled"}}"#)
    );
    assert_eq!(
        mapped("{}", r#"{"reasoning_effort":"none"}"#),
        value(r#"{"thinking":{"type":"disabled"}}"#)
    );
}

#[test]
fn a_null_thinking_object_falls_back_to_the_effort() {
    assert_eq!(
        mapped(r#"{"thinking":null,"reasoning_effort":"none"}"#, "{}"),
        value(r#"{"thinking":{"type":"disabled"}}"#)
    );
}

#[test]
fn a_requested_thinking_object_replaces_a_stale_one() {
    assert_eq!(
        mapped(
            r#"{"thinking":{"type":"disabled"}}"#,
            r#"{"thinking":{"type":"enabled"}}"#
        ),
        value(r#"{"thinking":{"type":"disabled"}}"#)
    );
}

#[test]
fn thinking_is_active_only_with_support_and_an_enabled_object() {
    let enabled = object(r#"{"thinking":{"type":"enabled"}}"#);
    let disabled = object(r#"{"thinking":{"type":"disabled"}}"#);
    let missing = object("{}");
    assert!(thinking_active(true, &enabled));
    assert!(!thinking_active(false, &enabled));
    assert!(!thinking_active(true, &disabled));
    assert!(!thinking_active(true, &missing));
}

#[test]
fn a_stored_reasoning_value_is_promoted_and_removed_from_the_nested_fields() {
    assert_eq!(
        repaired(
            r#"[{"role":"assistant","content":"hi","provider_specific_fields":{"reasoning_content":"thought","trace":"t"}}]"#
        ),
        value(
            r#"[{"role":"assistant","content":"hi","reasoning_content":"thought","provider_specific_fields":{"trace":"t"}}]"#
        )
    );
}

#[test]
fn the_promoted_nested_object_stays_even_when_it_becomes_empty() {
    assert_eq!(
        repaired(
            r#"[{"role":"assistant","content":"hi","provider_specific_fields":{"reasoning_content":"thought"}}]"#
        ),
        value(
            r#"[{"role":"assistant","content":"hi","reasoning_content":"thought","provider_specific_fields":{}}]"#
        )
    );
}

#[test]
fn a_missing_reasoning_value_becomes_a_space_with_one_warning() {
    let log = capture_logs(|| {
        let patched = repaired(r#"[{"role":"assistant","content":"hi"}]"#);
        assert_eq!(
            patched,
            value(r#"[{"role":"assistant","content":"hi","reasoning_content":" "}]"#)
        );
    });
    assert_eq!(occurrences(&log, "assistant message is missing"), 1);
}

#[test]
fn a_successful_promotion_does_not_warn() {
    let log = capture_logs(|| {
        let _patched = repaired(
            r#"[{"role":"assistant","content":"hi","provider_specific_fields":{"reasoning_content":"thought"}}]"#,
        );
    });
    assert_eq!(occurrences(&log, "assistant message is missing"), 0);
}

#[test]
fn an_empty_reasoning_value_counts_as_missing() {
    assert_eq!(
        repaired(
            r#"[{"role":"assistant","content":"hi","reasoning_content":"","provider_specific_fields":{"reasoning_content":"stored"}}]"#
        ),
        value(
            r#"[{"role":"assistant","content":"hi","reasoning_content":"stored","provider_specific_fields":{}}]"#
        )
    );
}

#[test]
fn a_falsy_stored_value_is_not_promoted() {
    let log = capture_logs(|| {
        let patched = repaired(
            r#"[{"role":"assistant","content":"hi","provider_specific_fields":{"reasoning_content":""}}]"#,
        );
        assert_eq!(
            patched,
            value(
                r#"[{"role":"assistant","content":"hi","reasoning_content":" ","provider_specific_fields":{"reasoning_content":""}}]"#
            )
        );
    });
    assert_eq!(occurrences(&log, "assistant message is missing"), 1);
}

#[test]
fn a_truthy_reasoning_value_is_left_alone() {
    let log = capture_logs(|| {
        let patched = repaired(
            r#"[{"role":"assistant","content":"hi","reasoning_content":"kept","provider_specific_fields":{"reasoning_content":"other"}}]"#,
        );
        assert_eq!(
            patched,
            value(
                r#"[{"role":"assistant","content":"hi","reasoning_content":"kept","provider_specific_fields":{"reasoning_content":"other"}}]"#
            )
        );
    });
    assert_eq!(occurrences(&log, "assistant message is missing"), 0);
}

#[test]
fn other_roles_are_left_alone() {
    assert_eq!(
        repaired(r#"[{"role":"user","content":"hi"}]"#),
        value(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn every_repair_warns_once() {
    let log = capture_logs(|| {
        let patched = repaired(
            r#"[{"role":"assistant","content":"a"},{"role":"user","content":"u"},{"role":"assistant","content":"b"}]"#,
        );
        assert_eq!(
            patched,
            value(
                r#"[{"role":"assistant","content":"a","reasoning_content":" "},{"role":"user","content":"u"},{"role":"assistant","content":"b","reasoning_content":" "}]"#
            )
        );
    });
    assert_eq!(occurrences(&log, "assistant message is missing"), 2);
}

#[test]
fn a_user_vision_list_is_forwarded_as_is() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
            true
        ),
        value(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#
        )
    );
}

#[test]
fn the_vision_gate_collapses_when_the_model_has_no_vision() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
            false
        ),
        value(r#"[{"role":"user","content":"look"}]"#)
    );
}

#[test]
fn a_text_only_list_collapses_even_with_vision() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]}]"#,
            true
        ),
        value(r#"[{"role":"user","content":"onetwo"}]"#)
    );
}

#[test]
fn an_image_url_object_with_a_url_is_forwardable() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#,
            true
        ),
        value(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#
        )
    );
}

#[rstest]
#[case::empty_url(r#""""#)]
#[case::no_url(r#"{"detail":"high"}"#)]
#[case::number_url(r#"{"url":5}"#)]
#[case::empty_object_url(r#"{"url":""}"#)]
#[case::numeric_payload("5")]
#[case::null_payload("null")]
fn a_malformed_image_url_falls_back_to_the_collapse(#[case] payload: &str) {
    let input = format!(
        r#"[{{"role":"user","content":[{{"type":"text","text":"look"}},{{"type":"image_url","image_url":{payload}}}]}}]"#
    );
    assert_eq!(
        prepared(&input, true),
        value(r#"[{"role":"user","content":"look"}]"#)
    );
}

#[rstest]
#[case::unknown_type(r#"[{"type":"text","text":"a"},{"type":"audio","audio":"x"}]"#)]
#[case::non_string_text(r#"[{"type":"text","text":"a"},{"type":"text","text":5}]"#)]
#[case::missing_text(r#"[{"type":"text","text":"a"},{"type":"text"}]"#)]
fn a_non_forwardable_block_falls_back_to_the_collapse(#[case] content: &str) {
    let input = format!(r#"[{{"role":"user","content":{content}}}]"#);
    assert_eq!(
        prepared(&input, true),
        value(r#"[{"role":"user","content":"a"}]"#)
    );
}

#[test]
fn a_non_user_list_is_never_forwarded() {
    assert_eq!(
        prepared(
            r#"[{"role":"assistant","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
            true
        ),
        value(r#"[{"role":"assistant","content":"look"}]"#)
    );
}

#[test]
fn a_textless_list_is_retained() {
    let textless = r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#;
    assert_eq!(prepared(textless, false), value(textless));
}

#[test]
fn an_identical_string_content_is_retained() {
    assert_eq!(
        prepared(r#"[{"role":"user","content":"same"}]"#, false),
        value(r#"[{"role":"user","content":"same"}]"#)
    );
}

#[test]
fn search_results_text_is_appended_as_a_final_text_block() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}],"search_results":[{"title":"T","content":[{"type":"text","text":"B"}]}]}]"#,
            true
        ),
        value(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"},{"type":"text","text":"TB"}]}]"#
        )
    );
}

#[test]
fn an_empty_search_extraction_keeps_the_original_message() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png"}],"search_results":[]}]"#,
            true
        ),
        value(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png"}],"search_results":[]}]"#
        )
    );
}

#[test]
fn search_results_text_folds_into_the_string_collapse() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":"hi","search_results":[{"source":"S","title":"T"}]}]"#,
            false
        ),
        value(
            r#"[{"role":"user","content":"hiST","search_results":[{"source":"S","title":"T"}]}]"#
        )
    );
}

#[test]
fn a_textless_list_with_search_text_becomes_the_search_text() {
    assert_eq!(
        prepared(
            r#"[{"role":"user","content":[],"search_results":[{"title":"T"}]}]"#,
            false
        ),
        value(r#"[{"role":"user","content":"T","search_results":[{"title":"T"}]}]"#)
    );
}

#[test]
fn transform_messages_applies_the_parent_content_transform() {
    let transformed = transformed_messages(
        "deepseek-chat",
        &chats(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://example.test/a.png","format":"png"}}]}]"#,
        ),
        true,
    );
    assert_eq!(
        transformed,
        value(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#
        )
    );
}

#[test]
fn transform_messages_collapses_when_vision_is_off() {
    let transformed = transformed_messages(
        "deepseek-chat",
        &chats(
            r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
        ),
        false,
    );
    assert_eq!(transformed, value(r#"[{"role":"user","content":"look"}]"#));
}

#[test]
fn transform_messages_wraps_a_forwarded_image_url_string() {
    let transformed = transformed_messages(
        "deepseek-chat",
        &chats(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
        ),
        true,
    );
    assert_eq!(
        transformed,
        value(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#
        )
    );
}

#[test]
fn mixed_tools_keep_functions_and_warn() {
    let (params, dropped_types) = dropped(
        r#"{"tools":[{"type":"function","function":{"name":"f"}},{"type":"namespace"}],"tool_choice":{"type":"function","function":{"name":"f"}},"parallel_tool_calls":true,"temperature":1}"#,
    );
    assert_eq!(
        params,
        value(
            r#"{"tool_choice":{"type":"function","function":{"name":"f"}},"parallel_tool_calls":true,"temperature":1,"tools":[{"type":"function","function":{"name":"f"}}]}"#
        )
    );
    assert_eq!(dropped_types, vec!["namespace".to_owned()]);
}

#[test]
fn a_choice_naming_a_dropped_tool_is_removed() {
    let (params, _) = dropped(
        r#"{"tools":[{"type":"function","function":{"name":"f"}},{"type":"namespace","name":"n"}],"tool_choice":{"type":"function","function":{"name":"namespace-tool"}},"parallel_tool_calls":true}"#,
    );
    assert_eq!(
        params,
        value(
            r#"{"parallel_tool_calls":true,"tools":[{"type":"function","function":{"name":"f"}}]}"#
        )
    );
}

#[test]
fn a_string_choice_survives_with_surviving_functions() {
    let (params, _) = dropped(
        r#"{"tools":[{"type":"function","function":{"name":"f"}},{"type":"namespace"}],"tool_choice":"auto"}"#,
    );
    assert_eq!(
        params,
        value(r#"{"tool_choice":"auto","tools":[{"type":"function","function":{"name":"f"}}]}"#)
    );
}

#[rstest]
#[case::wrong_type(r#"{"type":"custom"}"#)]
#[case::function_not_an_object(r#"{"type":"function","function":"f"}"#)]
#[case::name_not_a_string(r#"{"type":"function","function":{"name":5}}"#)]
#[case::unknown_name(r#"{"type":"function","function":{"name":"missing"}}"#)]
fn a_dangling_function_choice_is_removed(#[case] choice: &str) {
    let input = format!(
        r#"{{"tools":[{{"type":"function","function":{{"name":"f"}}}},{{"type":"namespace"}}],"tool_choice":{choice}}}"#
    );
    let (params, _) = dropped(&input);
    assert_eq!(
        params,
        value(r#"{"tools":[{"type":"function","function":{"name":"f"}}]}"#)
    );
}

#[test]
fn no_surviving_functions_drop_tools_choice_and_parallel_calls() {
    let (params, dropped_types) = dropped(
        r#"{"tools":[{"type":"namespace"},{"type":"web_search","name":"w"}],"tool_choice":"auto","parallel_tool_calls":true,"temperature":1}"#,
    );
    assert_eq!(params, value(r#"{"temperature":1}"#));
    assert_eq!(
        dropped_types,
        vec!["namespace".to_owned(), "web_search".to_owned()]
    );
}

#[test]
fn all_function_tools_skip_choice_cleanup() {
    let input = r#"{"tools":[{"type":"function","function":{"name":"f"}}],"tool_choice":{"type":"function","function":{"name":"missing"}},"parallel_tool_calls":true}"#;
    let (params, dropped_types) = dropped(input);
    assert_eq!(params, value(input));
    assert!(dropped_types.is_empty());
}

#[test]
fn empty_and_non_list_tools_are_unchanged() {
    let empty = r#"{"tools":[],"tool_choice":{"type":"function","function":{"name":"x"}}}"#;
    assert_eq!(dropped(empty).0, value(empty));
    let mapping = r#"{"tools":{"type":"namespace"},"tool_choice":"auto"}"#;
    assert_eq!(dropped(mapping).0, value(mapping));
}

#[test]
fn function_entries_without_a_name_still_survive() {
    let (params, _) = dropped(
        r#"{"tools":[{"type":"function","function":{}},{"type":"namespace"}],"tool_choice":{"type":"function","function":{"name":"f"}}}"#,
    );
    assert_eq!(
        params,
        value(r#"{"tools":[{"type":"function","function":{}}]}"#)
    );
}

#[rstest]
#[case::null("null", "null")]
#[case::boolean("false", "boolean")]
#[case::number("7", "number")]
#[case::string(r#""x""#, "string")]
#[case::list("[1]", "array")]
#[case::type_missing(r#"{"name":"n"}"#, "none")]
#[case::type_not_a_string(r#"{"type":7}"#, "number")]
fn dropped_type_names_describe_every_shape(#[case] tool: &str, #[case] name: &str) {
    let input = format!(r#"{{"tools":[{tool},{{"type":"namespace"}}]}}"#);
    let (_, dropped_types) = dropped(&input);
    let mut expected = vec![name.to_owned(), "namespace".to_owned()];
    expected.sort();
    assert_eq!(dropped_types, expected);
}

#[test]
fn dropped_type_names_are_sorted_and_distinct() {
    let (_, dropped_types) =
        dropped(r#"{"tools":[{"type":"zeta"},{"type":"alpha"},{"type":"zeta"},{"type":"alpha"}]}"#);
    assert_eq!(dropped_types, vec!["alpha".to_owned(), "zeta".to_owned()]);
}

#[test]
fn a_dropped_tool_warns_once() {
    let log = capture_logs(|| {
        let _outcome = dropped(r#"{"tools":[{"type":"namespace"},{"type":"web_search"}]}"#);
    });
    assert_eq!(occurrences(&log, "only supports function tools"), 1);
    assert!(log.contains("namespace"));
}

#[derive(Clone, Copy)]
struct Capabilities {
    vision: bool,
    reasoning: bool,
}

fn request_input<'request>(
    messages: &'request [ChatMessage],
    optional_params: &'request Object,
    capabilities: Capabilities,
) -> RequestInput<'request> {
    RequestInput {
        model: "deepseek-reasoner",
        messages,
        optional_params,
        supports_vision: capabilities.vision,
        supports_reasoning: capabilities.reasoning,
    }
}

#[test]
fn a_request_repairs_history_only_when_thinking_is_enabled() {
    let typed = chats(
        r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"done","provider_specific_fields":{"reasoning_content":"stored"}}]"#,
    );
    let body = transform_request(
        &request_input(
            &typed,
            &object(r#"{"thinking":{"type":"enabled"}}"#),
            Capabilities {
                vision: false,
                reasoning: true,
            },
        ),
        Mode::Sync,
    )
    .expect("the request");
    assert_eq!(
        rendered(&body)["messages"],
        value(
            r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"done","reasoning_content":"stored","provider_specific_fields":{}}]"#
        )
    );
}

#[rstest]
#[case::model_without_reasoning(false, r#"{"thinking":{"type":"enabled"}}"#)]
#[case::thinking_disabled(true, r#"{"thinking":{"type":"disabled"}}"#)]
#[case::thinking_absent(true, "{}")]
#[case::thinking_malformed(true, r#"{"thinking":{"type":"auto"}}"#)]
fn a_request_leaves_history_alone_without_active_thinking(
    #[case] supports_reasoning: bool,
    #[case] optional_params: &str,
) {
    let typed = chats(r#"[{"role":"assistant","content":"done"}]"#);
    let body = transform_request(
        &request_input(
            &typed,
            &object(optional_params),
            Capabilities {
                vision: false,
                reasoning: supports_reasoning,
            },
        ),
        Mode::Sync,
    )
    .expect("the request");
    assert_eq!(
        rendered(&body)["messages"],
        value(r#"[{"role":"assistant","content":"done"}]"#)
    );
}

#[test]
fn a_request_drops_unsupported_tools_before_repairing_history() {
    let typed = chats(r#"[{"role":"assistant","content":"done"}]"#);
    let optional = object(r#"{"tools":[{"type":"namespace"}],"thinking":{"type":"enabled"}}"#);
    let log = capture_logs(|| {
        let body = transform_request(
            &request_input(
                &typed,
                &optional,
                Capabilities {
                    vision: false,
                    reasoning: true,
                },
            ),
            Mode::Sync,
        )
        .expect("the request");
        assert_eq!(rendered(&body)["thinking"], value(r#"{"type":"enabled"}"#));
        assert!(
            !rendered(&body)
                .as_object()
                .expect("a body")
                .contains_key("tools")
        );
    });
    let tools = log
        .find("only supports function tools")
        .expect("the tools warning");
    let history = log
        .find("assistant message is missing")
        .expect("the history warning");
    assert!(tools < history);
}

#[test]
fn a_request_forwards_vision_content_only_with_the_capability() {
    let typed = chats(
        r#"[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
    );
    let empty = Object::new();
    let forwarded = transform_request(
        &request_input(
            &typed,
            &empty,
            Capabilities {
                vision: true,
                reasoning: false,
            },
        ),
        Mode::Sync,
    )
    .expect("the request");
    assert_eq!(
        rendered(&forwarded)["messages"][0]["content"][1]["image_url"],
        value(r#"{"url":"https://example.test/a.png"}"#)
    );
    let collapsed = transform_request(
        &request_input(
            &typed,
            &empty,
            Capabilities {
                vision: false,
                reasoning: false,
            },
        ),
        Mode::Sync,
    )
    .expect("the request");
    assert_eq!(
        rendered(&collapsed)["messages"][0]["content"],
        value(r#""look""#)
    );
}

#[test]
fn sync_and_async_requests_produce_the_same_body_and_warnings() {
    let typed = chats(r#"[{"role":"assistant","content":"done"}]"#);
    let optional = object(
        r#"{"tools":[{"type":"namespace"}],"thinking":{"type":"enabled"},"temperature":0.2}"#,
    );
    let input = request_input(
        &typed,
        &optional,
        Capabilities {
            vision: false,
            reasoning: true,
        },
    );
    let sync_log = capture_logs(|| {
        let _body = transform_request(&input, Mode::Sync).expect("the sync request");
    });
    let async_log = capture_logs(|| {
        let _body = transform_request(&input, Mode::AsyncSubclass).expect("the async request");
    });
    assert_eq!(occurrences(&sync_log, "only supports function tools"), 1);
    assert_eq!(occurrences(&async_log, "only supports function tools"), 1);
    assert_eq!(occurrences(&sync_log, "assistant message is missing"), 1);
    assert_eq!(occurrences(&async_log, "assistant message is missing"), 1);
    assert_eq!(
        transform_request(&input, Mode::Sync).expect("the sync request"),
        transform_request(&input, Mode::AsyncSubclass).expect("the async request")
    );
}

#[rstest]
#[case::absent(None, "https://api.deepseek.com/beta/chat/completions")]
#[case::empty(Some(""), "https://api.deepseek.com/beta/chat/completions")]
#[case::custom(
    Some("https://custom.example"),
    "https://custom.example/chat/completions"
)]
#[case::already_complete(
    Some("https://custom.example/chat/completions"),
    "https://custom.example/chat/completions"
)]
#[case::trailing_slash(
    Some("https://custom.example/"),
    "https://custom.example//chat/completions"
)]
fn complete_url_uses_the_truthy_base_and_appends_the_suffix(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(api_base), expected);
}

struct FakePricing {
    seen: Mutex<Option<(String, CostUsage, Option<String>)>>,
}

impl GenericPricing for FakePricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        usage: &CostUsage,
        provider: Option<&str>,
    ) -> Result<GenericCostResult, TokenCostError> {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((model.to_owned(), usage.clone(), provider.map(str::to_owned)));
        Ok(GenericCostResult {
            prompt_cost: Decimal::ONE,
            completion_cost: Decimal::TWO,
        })
    }
}

#[test]
fn the_cost_wrapper_forwards_the_model_usage_and_provider_tag() {
    let pricing = FakePricing {
        seen: Mutex::new(None),
    };
    let usage = CostUsage {
        prompt_tokens: 3,
        completion_tokens: 4,
        total_tokens: 7,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: Some(1),
        numeric_extras: BTreeMap::new(),
    };
    let cost = pricing
        .cost_per_token("deepseek-reasoner", &usage)
        .expect("the forwarded cost");
    assert_eq!(cost.prompt_cost, Decimal::ONE);
    assert_eq!(cost.completion_cost, Decimal::TWO);
    let seen = pricing
        .seen
        .lock()
        .expect("the fake lock")
        .clone()
        .expect("a recorded call");
    assert_eq!(seen.0, "deepseek-reasoner");
    assert_eq!(seen.1, usage);
    assert_eq!(seen.2.as_deref(), Some("deepseek"));
}
