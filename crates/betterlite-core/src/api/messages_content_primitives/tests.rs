#![cfg(test)]

use std::future::{Future, ready};

use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::markers::{
    carry_cache_breakpoint, copy_cache_control, mid_system_supported, target_consumes_cache_control,
};
use super::signatures::{
    ToolCallFields, compaction_blocks, tool_call_signature, tool_use_signature,
};
use super::tool_results::{
    ToolResultContent, ToolResultPart, image_source_url, tool_result_content,
    tool_result_image_part, tool_result_part,
};
use super::tools::{
    ChatToolParam, TRANSLATABLE_ANTHROPIC_PARAMS, chat_tool_param, is_openai_function_tool,
    is_provider_native_tool_dict, is_web_search_tool, truncate_tool_name,
};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("fixture literal is a JSON object")
}

#[rstest]
#[case::web_search_dict(r#"{"web_search": {}}"#, true)]
#[case::empty_dict("{}", false)]
#[case::two_keys(r#"{"web_search": {}, "other": {}}"#, false)]
#[case::scalar_value(r#"{"web_search": true}"#, false)]
#[case::null_value(r#"{"web_search": null}"#, false)]
#[case::name_key(r#"{"name": {}}"#, false)]
#[case::type_key(r#"{"type": {}}"#, false)]
#[case::input_schema_key(r#"{"input_schema": {}}"#, false)]
#[case::description_key(r#"{"description": {}}"#, false)]
#[case::cache_control_key(r#"{"cache_control": {}}"#, false)]
#[case::strict_key(r#"{"strict": {}}"#, false)]
fn native_tool_dict_needs_one_dict_valued_key_outside_the_schema_keys(
    #[case] tool: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_provider_native_tool_dict(&object(tool)), expected);
}

#[rstest]
#[case::null_function(r#"{"type": "function", "function": null}"#, true)]
#[case::dict_function(r#"{"type": "function", "function": {"name": "a"}}"#, true)]
#[case::no_function_key(r#"{"type": "function"}"#, false)]
#[case::custom_type(r#"{"type": "custom", "function": {}}"#, false)]
#[case::no_type_key(r#"{"function": {}}"#, false)]
#[case::non_string_type(r#"{"type": 7, "function": {}}"#, false)]
fn openai_function_tool_needs_the_function_type_and_key(
    #[case] tool: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_openai_function_tool(&object(tool)), expected);
}

#[rstest]
#[case::dated_type(r#"{"type": "web_search_20260209"}"#, true)]
#[case::bare_type(r#"{"type": "web_search"}"#, true)]
#[case::legacy_name(r#"{"name": "web_search"}"#, true)]
#[case::name_with_schema(r#"{"name": "web_search", "input_schema": {}}"#, false)]
#[case::function_type(r#"{"type": "function"}"#, false)]
#[case::null_type_with_legacy_name(r#"{"type": null, "name": "web_search"}"#, true)]
#[case::other_name(r#"{"name": "web_search_2"}"#, false)]
fn web_search_tool_is_a_typed_prefix_or_the_legacy_name_without_a_schema(
    #[case] tool: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_web_search_tool(&object(tool)), expected);
}

#[rstest]
#[case::empty(String::new(), String::new())]
#[case::exactly_64("a".repeat(64), "a".repeat(64))]
#[case::multibyte_at_the_limit("\u{e9}".repeat(64), "\u{e9}".repeat(64))]
#[case::one_over_the_limit("a".repeat(65), format!("{}_635361c4", "a".repeat(55)))]
#[case::same_prefix_other_suffix(
    format!("{}{}", "a".repeat(55), "b".repeat(10)),
    format!("{}_41b2c7ec", "a".repeat(55))
)]
#[case::multibyte_over_the_limit("\u{e9}".repeat(65), format!("{}_c8a2666a", "\u{e9}".repeat(55)))]
#[case::hash_covers_the_utf8_name(
    format!("{}{}", "a".repeat(55), "\u{e9}".repeat(10)),
    format!("{}_3e1dcffe", "a".repeat(55))
)]
fn truncate_tool_name_keeps_64_characters_and_hashes_longer_names(
    #[case] name: String,
    #[case] expected: String,
) {
    assert_eq!(truncate_tool_name(&name), expected);
}

#[test]
fn translatable_params_are_the_nine_adapter_keys_in_order() {
    assert_eq!(
        TRANSLATABLE_ANTHROPIC_PARAMS,
        [
            "messages",
            "metadata",
            "system",
            "tool_choice",
            "tools",
            "thinking",
            "output_format",
            "output_config",
            "stop_sequences",
        ]
    );
}

fn tool_call(text: &str) -> ToolCallFields {
    serde_json::from_str(text).expect("fixture literal is a tool call")
}

#[rstest]
#[case::outer_wins(
    r#"{"provider_specific_fields": {"thought_signature": "outer"}, "function": {"provider_specific_fields": {"thought_signature": "inner"}}}"#,
    Some("outer")
)]
#[case::truthy_outer_without_signature_hides_inner(
    r#"{"provider_specific_fields": {"other": "x"}, "function": {"provider_specific_fields": {"thought_signature": "inner"}}}"#,
    None
)]
#[case::empty_outer_falls_back(
    r#"{"provider_specific_fields": {}, "function": {"provider_specific_fields": {"thought_signature": "inner"}}}"#,
    Some("inner")
)]
#[case::absent_outer_falls_back(
    r#"{"function": {"provider_specific_fields": {"thought_signature": "inner"}}}"#,
    Some("inner")
)]
#[case::non_string_signature(r#"{"provider_specific_fields": {"thought_signature": 7}}"#, None)]
#[case::empty_signature(r#"{"provider_specific_fields": {"thought_signature": ""}}"#, Some(""))]
#[case::no_fields("{}", None)]
fn tool_call_signature_reads_the_outer_mapping_before_the_function_mapping(
    #[case] call: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(tool_call_signature(&tool_call(call)), expected);
}

#[rstest]
#[case::signature(r#"{"provider_specific_fields": {"signature": "sig"}}"#, Some("sig"))]
#[case::empty_signature(r#"{"provider_specific_fields": {"signature": ""}}"#, Some(""))]
#[case::ignores_thought_signature(
    r#"{"provider_specific_fields": {"thought_signature": "sig"}}"#,
    None
)]
#[case::missing_fields("{}", None)]
#[case::non_mapping_fields(r#"{"provider_specific_fields": "sig"}"#, None)]
#[case::non_string_signature(r#"{"provider_specific_fields": {"signature": 7}}"#, None)]
fn tool_use_signature_reads_only_the_signature_key(
    #[case] content: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(tool_use_signature(&object(content)), expected);
}

#[rstest]
#[case::keeps_compaction_blocks_in_order(
    r#"{"compaction_blocks": [{"type": "compaction", "n": 1}, "text", {"type": "other"}, {"type": "compaction", "n": 2}]}"#,
    r#"[{"type": "compaction", "n": 1}, {"type": "compaction", "n": 2}]"#
)]
#[case::absent("{}", "[]")]
#[case::null(r#"{"compaction_blocks": null}"#, "[]")]
#[case::dict(r#"{"compaction_blocks": {"type": "compaction"}}"#, "[]")]
#[case::scalar(r#"{"compaction_blocks": 7}"#, "[]")]
#[case::block_without_type(r#"{"compaction_blocks": [{}, {"type": 7}]}"#, "[]")]
fn compaction_blocks_keep_only_compaction_mappings_of_a_list(
    #[case] fields: &str,
    #[case] expected: &str,
) {
    let blocks: Vec<Json> = compaction_blocks(&object(fields))
        .into_iter()
        .map(|block| Json::Object(block.clone()))
        .collect();
    assert_eq!(Json::List(blocks), json(expected));
}

#[rstest]
#[case::anthropic_prefix("anthropic/claude-3-opus", true)]
#[case::gemini_prefix("gemini/gemini-2.5-pro", true)]
#[case::bedrock_arn("arn:aws:bedrock:us-east-1:123:application-inference-profile/id", true)]
#[case::sagemaker_endpoint_naming_bedrock(
    "arn:aws:sagemaker:us-east-1:123:endpoint/my-bedrock-endpoint",
    false
)]
#[case::openai("openai/gpt-4o", false)]
#[case::empty("", false)]
#[case::upper_case_anthropic("ANTHROPIC/x", true)]
#[case::upper_case_claude("Vertex/CLAUDE-x", true)]
#[case::upper_case_gemini("GEMINI-x", true)]
#[case::upper_case_bedrock_arn("ARN:aws:BEDROCK:us-east-1:1:x", true)]
#[case::arn_without_bedrock_service("arn:aws:s3:::bucket", false)]
#[case::bedrock_service_without_arn("my:bedrock:model", false)]
fn cache_control_is_consumed_by_claude_bedrock_arn_and_gemini_targets(
    #[case] model: &str,
    #[case] expected: bool,
) {
    assert_eq!(target_consumes_cache_control(model), expected);
}

fn copied_cache_control(source: &str, model: Option<&str>) -> Object {
    let mut target = object(r#"{"type": "text"}"#);
    copy_cache_control(&object(source), &mut target, model);
    target
}

const EPHEMERAL: &str = r#"{"cache_control": {"type": "ephemeral", "ttl": "1h"}}"#;
const COPIED: &str = r#"{"type": "text", "cache_control": {"type": "ephemeral", "ttl": "1h"}}"#;

#[rstest]
#[case::anthropic_model(EPHEMERAL, Some("anthropic/claude-3-opus"), COPIED)]
#[case::gemini_model(EPHEMERAL, Some("gemini/gemini-2.5-pro"), COPIED)]
#[case::bedrock_arn(
    EPHEMERAL,
    Some("arn:aws:bedrock:us-east-1:123:application-inference-profile/id"),
    COPIED
)]
#[case::sagemaker_arn(
    EPHEMERAL,
    Some("arn:aws:sagemaker:us-east-1:123:endpoint/my-bedrock-endpoint"),
    r#"{"type": "text"}"#
)]
#[case::openai_model(EPHEMERAL, Some("openai/gpt-4o"), r#"{"type": "text"}"#)]
#[case::no_model(EPHEMERAL, None, r#"{"type": "text"}"#)]
#[case::empty_model(EPHEMERAL, Some(""), r#"{"type": "text"}"#)]
#[case::empty_marker(r#"{"cache_control": {}}"#, Some("claude-3"), r#"{"type": "text"}"#)]
#[case::null_marker(r#"{"cache_control": null}"#, Some("claude-3"), r#"{"type": "text"}"#)]
#[case::absent_marker("{}", Some("claude-3"), r#"{"type": "text"}"#)]
fn cache_control_is_copied_for_a_truthy_marker_and_an_accepted_model(
    #[case] source: &str,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(copied_cache_control(source, model), object(expected));
}

#[rstest]
#[case::absent("{}", r#"{"role": "user"}"#)]
#[case::null(r#"{"prompt_cache_breakpoint": null}"#, r#"{"role": "user"}"#)]
#[case::false_marker(
    r#"{"prompt_cache_breakpoint": false}"#,
    r#"{"role": "user", "prompt_cache_breakpoint": false}"#
)]
#[case::zero_marker(
    r#"{"prompt_cache_breakpoint": 0}"#,
    r#"{"role": "user", "prompt_cache_breakpoint": 0}"#
)]
#[case::text_marker(
    r#"{"prompt_cache_breakpoint": "mark"}"#,
    r#"{"role": "user", "prompt_cache_breakpoint": "mark"}"#
)]
fn prompt_cache_breakpoint_is_selected_by_key_and_dropped_only_when_null(
    #[case] source: &str,
    #[case] expected: &str,
) {
    let target = object(r#"{"role": "user"}"#);
    let marked = carry_cache_breakpoint(&object(source), &target);
    assert_eq!(marked, object(expected));
}

struct Recorder {
    answer: bool,
    calls: Vec<(String, Option<String>, String)>,
}

impl CapabilityPredicates for Recorder {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.calls.push((
            request.model.to_owned(),
            request.custom_llm_provider.map(str::to_owned),
            key.to_owned(),
        ));
        ready(self.answer)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = DecisionJson> + Send {
        ready(DecisionJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

#[rstest]
#[case::no_model(None)]
#[case::empty_model(Some(""))]
#[tokio::test]
async fn mid_conversation_system_skips_the_predicate_without_a_model(#[case] model: Option<&str>) {
    let mut predicates = Recorder {
        answer: true,
        calls: Vec::new(),
    };
    let supported = mid_system_supported(&mut predicates, model, Some("anthropic")).await;
    assert!(!supported);
    assert!(predicates.calls.is_empty());
}

#[rstest]
#[case::supported(true)]
#[case::unsupported(false)]
#[tokio::test]
async fn mid_conversation_system_forwards_both_arguments_and_the_answer(#[case] answer: bool) {
    let mut predicates = Recorder {
        answer,
        calls: Vec::new(),
    };
    let supported =
        mid_system_supported(&mut predicates, Some("claude-3-opus"), Some("anthropic")).await;
    assert_eq!(supported, answer);
    assert_eq!(
        predicates.calls,
        [(
            "claude-3-opus".to_owned(),
            Some("anthropic".to_owned()),
            "supports_mid_conversation_system".to_owned()
        )]
    );
}

#[tokio::test]
async fn mid_conversation_system_forwards_a_missing_provider() {
    let mut predicates = Recorder {
        answer: true,
        calls: Vec::new(),
    };
    mid_system_supported(&mut predicates, Some("claude-3-opus"), None).await;
    assert_eq!(
        predicates.calls.first().map(|call| call.1.clone()),
        Some(None)
    );
}

fn function_chunk() -> Object {
    object(r#"{"name": "lookup"}"#)
}

#[rstest]
#[case::absent_flag(r#"{"name": "lookup"}"#, None)]
#[case::null_flag(r#"{"eager_input_streaming": null}"#, None)]
#[case::top_level_false_beats_nested_true(
    r#"{"eager_input_streaming": false, "function": {"eager_input_streaming": true}}"#,
    Some(false)
)]
#[case::top_level_true(r#"{"eager_input_streaming": true}"#, Some(true))]
#[case::nested_true(r#"{"function": {"eager_input_streaming": true}}"#, Some(true))]
#[case::non_mapping_tool("7", None)]
fn chat_tool_param_carries_the_eager_flag_only_when_one_is_set(
    #[case] tool: &str,
    #[case] expected: Option<bool>,
) {
    let param = chat_tool_param(function_chunk(), &json(tool)).expect("valid flag");
    assert_eq!(
        param,
        ChatToolParam {
            kind: "function",
            function: function_chunk(),
            eager_input_streaming: expected,
        }
    );
}

#[rstest]
#[case::flag_set(
    Some(true),
    r#"{"type":"function","function":{"name":"lookup"},"eager_input_streaming":true}"#
)]
#[case::flag_unset(None, r#"{"type":"function","function":{"name":"lookup"}}"#)]
fn chat_tool_param_serializes_the_flag_last_and_omits_it_when_unset(
    #[case] flag: Option<bool>,
    #[case] expected: &str,
) {
    let param = ChatToolParam {
        kind: "function",
        function: function_chunk(),
        eager_input_streaming: flag,
    };
    assert_eq!(serde_json::to_string(&param).expect("serializes"), expected);
}

#[rstest]
#[case::top_level_string(r#"{"eager_input_streaming": "true"}"#)]
#[case::nested_string(r#"{"function": {"eager_input_streaming": "true"}}"#)]
fn chat_tool_param_rejects_a_non_boolean_flag_with_a_400(#[case] tool: &str) {
    let error = chat_tool_param(function_chunk(), &json(tool)).expect_err("non-boolean flag");
    assert!(matches!(error, FeatureFlagsError::EagerInputStreaming));
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::base64_default_media_type(
    r#"{"type": "base64", "data": "YWJj"}"#,
    Some("data:image/jpeg;base64,YWJj")
)]
#[case::base64_pdf(
    r#"{"type": "base64", "media_type": "application/pdf", "data": "YWJj"}"#,
    Some("data:application/pdf;base64,YWJj")
)]
#[case::base64_without_data(r#"{"type": "base64"}"#, None)]
#[case::base64_empty_data(r#"{"type": "base64", "data": ""}"#, None)]
#[case::url(
    r#"{"type": "url", "url": "https://image.test/a"}"#,
    Some("https://image.test/a")
)]
#[case::non_string_url(r#"{"type": "url", "url": 7}"#, Some(""))]
#[case::unknown_type(r#"{"type": "file"}"#, None)]
#[case::non_dict_source(r#""base64""#, None)]
#[case::list_source("[]", None)]
fn image_source_projects_to_a_data_url_or_passes_the_url_through(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(image_source_url(&json(source)).as_deref(), expected);
}

fn image_part(url: &str) -> ToolResultPart {
    ToolResultPart::ImageUrl {
        image_url: super::tool_results::ImageUrl {
            url: url.to_owned(),
        },
    }
}

fn text_part(text: &str) -> ToolResultPart {
    ToolResultPart::Text {
        text: text.to_owned(),
    }
}

#[rstest]
#[case::image_block(
    r#"{"type": "image", "source": {"type": "url", "url": "https://i/a"}}"#,
    Some(image_part("https://i/a"))
)]
#[case::document_block(
    r#"{"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "YWJj"}}"#,
    Some(image_part("data:application/pdf;base64,YWJj"))
)]
#[case::string_item(r#""hello""#, Some(text_part("hello")))]
#[case::non_dict_item("7", None)]
#[case::list_item("[]", None)]
#[case::unknown_type(r#"{"type": "audio"}"#, None)]
#[case::missing_type("{}", None)]
#[case::text_block(r#"{"type": "text", "text": "a"}"#, Some(text_part("a")))]
#[case::null_text(r#"{"type": "text", "text": null}"#, Some(text_part("")))]
#[case::zero_text(r#"{"type": "text", "text": 0}"#, Some(text_part("")))]
#[case::absent_text(r#"{"type": "text"}"#, Some(text_part("")))]
#[case::number_text(r#"{"type": "text", "text": 7}"#, Some(text_part("7")))]
#[case::true_text(r#"{"type": "text", "text": true}"#, Some(text_part("true")))]
#[case::list_text(r#"{"type": "text", "text": [1]}"#, Some(text_part("[1]")))]
#[case::tool_reference(
    r#"{"type": "tool_reference", "tool_name": "lookup"}"#,
    Some(ToolResultPart::ToolReference { tool_name: "lookup".to_owned() })
)]
#[case::tool_reference_without_name(
    r#"{"type": "tool_reference"}"#,
    Some(ToolResultPart::ToolReference { tool_name: String::new() })
)]
#[case::tool_reference_null_name(
    r#"{"type": "tool_reference", "tool_name": null}"#,
    Some(ToolResultPart::ToolReference { tool_name: String::new() })
)]
#[case::image_without_source(r#"{"type": "image"}"#, None)]
#[case::image_with_invalid_source(r#"{"type": "image", "source": {"type": "file"}}"#, None)]
#[case::image_with_non_dict_source(r#"{"type": "image", "source": "x"}"#, None)]
#[case::image_with_empty_url(r#"{"type": "image", "source": {"type": "url", "url": 7}}"#, None)]
fn tool_result_part_maps_each_block_type(
    #[case] item: &str,
    #[case] expected: Option<ToolResultPart>,
) {
    assert_eq!(tool_result_part(&json(item)), expected);
}

#[test]
fn tool_result_image_part_needs_a_source() {
    assert_eq!(tool_result_image_part(None), None);
    assert_eq!(
        tool_result_image_part(Some(&json(r#"{"type": "url", "url": "https://i/a"}"#))),
        Some(image_part("https://i/a"))
    );
}

#[rstest]
#[case::string(r#""hello""#, ToolResultContent::Text("hello".to_owned()))]
#[case::null("null", ToolResultContent::Text(String::new()))]
#[case::number("7", ToolResultContent::Text(String::new()))]
#[case::dict("{}", ToolResultContent::Text(String::new()))]
#[case::empty_list("[]", ToolResultContent::Text(String::new()))]
#[case::every_entry_dropped(r#"[7, {"type": "audio"}]"#, ToolResultContent::Text(String::new()))]
#[case::one_text_collapses(r#"["a"]"#, ToolResultContent::Text("a".to_owned()))]
#[case::two_texts_stay_a_list(
    r#"["a", {"type": "text", "text": 7}]"#,
    ToolResultContent::Parts(vec![text_part("a"), text_part("7")])
)]
#[case::text_and_image_stay_a_list(
    r#"["a", {"type": "image", "source": {"type": "url", "url": "https://i/a"}}, {"type": "image", "source": {"type": "file"}}]"#,
    ToolResultContent::Parts(vec![text_part("a"), image_part("https://i/a")])
)]
#[case::one_image_stays_a_list(
    r#"[{"type": "image", "source": {"type": "url", "url": "https://i/a"}}]"#,
    ToolResultContent::Parts(vec![image_part("https://i/a")])
)]
#[case::one_tool_reference_stays_a_list(
    r#"[{"type": "tool_reference", "tool_name": "t"}]"#,
    ToolResultContent::Parts(vec![ToolResultPart::ToolReference { tool_name: "t".to_owned() }])
)]
fn tool_result_content_collapses_one_text_part_and_filters_the_rest(
    #[case] raw: &str,
    #[case] expected: ToolResultContent,
) {
    assert_eq!(tool_result_content(&json(raw)), expected);
}

#[test]
fn tool_result_parts_serialize_with_a_type_tag() {
    let content = ToolResultContent::Parts(vec![
        text_part("a"),
        image_part("https://i/a"),
        ToolResultPart::ToolReference {
            tool_name: "t".to_owned(),
        },
    ]);
    assert_eq!(
        serde_json::to_string(&content).expect("serializes"),
        r#"[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"https://i/a"}},{"type":"tool_reference","tool_name":"t"}]"#
    );
}
