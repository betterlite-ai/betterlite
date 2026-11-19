#![cfg(test)]

use std::future::{Future, ready};

use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{ChatTarget, translate_messages};

type Probe = (String, Option<String>);

struct Capabilities {
    supported: Vec<&'static str>,
    probes: Vec<Probe>,
}

impl CapabilityPredicates for Capabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.probes.push((
            request.model.to_owned(),
            request.custom_llm_provider.map(str::to_owned),
        ));
        ready(self.supported.contains(&request.model))
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

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn target(model: &str) -> ChatTarget<'_> {
    let provider = match model {
        "gpt-4o" => "openai",
        "gemini-2.0-flash" => "gemini",
        _ => "anthropic",
    };
    ChatTarget {
        model: Some(model),
        custom_llm_provider: Some(provider),
    }
}

async fn run(
    messages: &str,
    model: &str,
    preserve: bool,
    supported: &[&'static str],
) -> (Json, Vec<Probe>) {
    let mut capabilities = Capabilities {
        supported: supported.to_vec(),
        probes: Vec::new(),
    };
    let translated =
        translate_messages(&json(messages), &target(model), preserve, &mut capabilities)
            .await
            .expect("the fixture translates");
    (Json::List(translated), capabilities.probes)
}

#[rstest]
#[case::ordering(
    r#"[{"role":"user","content":[{"type":"text","text":"before"},{"type":"tool_result","tool_use_id":"t","content":"ok"},{"type":"text","text":"after"}]}]"#,
    r#"[{"role":"tool","tool_call_id":"t","content":"ok"},{"role":"user","content":[{"type":"text","text":"before"},{"type":"text","text":"after"}]}]"#
)]
#[case::absent_fields(
    r#"[{"role":"user","content":[{"type":"tool_result"}]}]"#,
    r#"[{"role":"tool","tool_call_id":"","content":""}]"#
)]
#[case::scalar_hello(
    r#"[{"role":"user","content":"hello"}]"#,
    r#"[{"role":"user","content":"hello"}]"#
)]
#[case::empty_string(r#"[{"role":"user","content":""}]"#, "[]")]
#[case::empty_list(r#"[{"role":"user","content":[]}]"#, "[]")]
#[case::null_content(r#"[{"role":"user","content":null}]"#, "[]")]
#[case::unknown_role(r#"[{"role":"other","content":"ignored"}]"#, "[]")]
#[case::unknown_block(r#"[{"role":"user","content":[{"type":"banana","text":"x"}]}]"#, "[]")]
#[case::missing_text(
    r#"[{"role":"user","content":[{"type":"text"}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":""}]}]"#
)]
#[case::is_error_not_forwarded(
    r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok","is_error":true}]}]"#,
    r#"[{"role":"tool","tool_call_id":"t","content":"ok"}]"#
)]
#[case::tool_result_image(
    r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":[{"type":"image","source":{"type":"url","url":"https://example.test/a.png"}}],"cache_control":{"type":"ephemeral"}}]}]"#,
    r#"[{"role":"tool","tool_call_id":"t","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}],"cache_control":{"type":"ephemeral"}}]"#
)]
#[case::non_object_entries(
    r#"[{"role":"user","content":["hello",{"type":"text","text":"kept"}]},"bare"]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"kept"}]}]"#
)]
#[case::non_string_values(
    r#"[{"role":"user","content":[{"type":"text","text":7},{"type":"tool_result","tool_use_id":9,"content":"ok"}]}]"#,
    r#"[{"role":"tool","tool_call_id":"","content":"ok"},{"role":"user","content":[{"type":"text","text":""}]}]"#
)]
#[tokio::test]
async fn user_and_tool_order(#[case] input: &str, #[case] expected: &str) {
    let (translated, probes) = run(input, "claude-sonnet-5", false, &[]).await;
    assert_eq!(translated, json(expected));
    assert!(probes.is_empty());
}

#[rstest]
#[case::claude_media(
    "claude-sonnet-5",
    r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"},"prompt_cache_breakpoint":false},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"YWJj"},"cache_control":{"type":"ephemeral"},"prompt_cache_breakpoint":false},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"YWJj"},"cache_control":{"type":"ephemeral"},"prompt_cache_breakpoint":false},{"type":"image","source":{"type":"url","url":"https://example.test/cat.png"}},{"type":"image","source":{"type":"nope"}},{"type":"document","source":{"type":"base64","media_type":"application/pdf"}}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"},"prompt_cache_breakpoint":false},{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,YWJj"},"cache_control":{"type":"ephemeral"},"prompt_cache_breakpoint":false},{"type":"image_url","image_url":{"url":"data:application/pdf;base64,YWJj"},"cache_control":{"type":"ephemeral"}},{"type":"image_url","image_url":{"url":"https://example.test/cat.png"}}]}]"#
)]
#[case::gemini_keeps_cache(
    "gemini-2.0-flash",
    r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"YWJj"},"cache_control":{"type":"ephemeral"}}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}},{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,YWJj"},"cache_control":{"type":"ephemeral"}}]}]"#
)]
#[case::gpt_strips_cache_keeps_breakpoint(
    "gpt-4o",
    r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"},"prompt_cache_breakpoint":false}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi","prompt_cache_breakpoint":false}]}]"#
)]
#[case::empty_url_media_dropped(
    "claude-sonnet-5",
    r#"[{"role":"user","content":[{"type":"image","source":{"type":"url","url":""}},{"type":"document","source":{"type":"url","url":""}}]}]"#,
    "[]"
)]
#[tokio::test]
async fn user_media_markers(#[case] model: &str, #[case] input: &str, #[case] expected: &str) {
    let (translated, probes) = run(input, model, false, &[]).await;
    assert_eq!(translated, json(expected));
    assert!(probes.is_empty());
}

#[rstest]
#[case::concatenated(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]"#,
    r#"[{"role":"assistant","content":"ab","thinking_blocks":null}]"#
)]
#[case::retained_cache_control_keeps_list(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b"}]}]"#,
    r#"[{"role":"assistant","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b"}],"thinking_blocks":null}]"#
)]
#[case::stripped_for_gpt(
    "gpt-4o",
    r#"[{"role":"assistant","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b"}]}]"#,
    r#"[{"role":"assistant","content":"ab","thinking_blocks":null}]"#
)]
#[case::strings_keep_last(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":["a","b"]}]"#,
    r#"[{"role":"assistant","content":"b","thinking_blocks":null}]"#
)]
#[case::text_dicts_beat_string(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":["c",{"type":"text","text":"d"}]}]"#,
    r#"[{"role":"assistant","content":"d","thinking_blocks":null}]"#
)]
#[case::empty_string_emits_empty_content(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":""}]"#,
    r#"[{"role":"assistant","content":"","thinking_blocks":null}]"#
)]
#[case::empty_list_emits_nothing("claude-sonnet-5", r#"[{"role":"assistant","content":[]}]"#, "[]")]
#[case::missing_content_emits_nothing("claude-sonnet-5", r#"[{"role":"assistant"}]"#, "[]")]
#[case::ignores_other_values(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[7,null,{"type":"text","text":"d"}]}]"#,
    r#"[{"role":"assistant","content":"d","thinking_blocks":null}]"#
)]
#[case::second_text_block_cache_control(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}]}]"#,
    r#"[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}],"thinking_blocks":null}]"#
)]
#[tokio::test]
async fn assistant_text_representation(
    #[case] model: &str,
    #[case] input: &str,
    #[case] expected: &str,
) {
    let (translated, probes) = run(input, model, false, &[]).await;
    assert_eq!(translated, json(expected));
    assert!(probes.is_empty());
}

#[rstest]
#[case::tool_use(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{"x":1}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{\"x\": 1}"}}]}]"#
)]
#[case::defaults(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use"}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"","type":"function","function":{"name":"","arguments":"{}"}}]}]"#
)]
#[case::signature(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{},"provider_specific_fields":{"signature":"sig"}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{}","provider_specific_fields":{"thought_signature":"sig"}}}]}]"#
)]
#[case::empty_signature_omitted(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{},"provider_specific_fields":{"signature":""}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{}"}}]}]"#
)]
#[case::long_name_truncated(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":"u","name":"nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn","input":{}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"u","type":"function","function":{"name":"nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn_1e3fb6d5","arguments":"{}"}}]}]"#
)]
#[case::tool_cache_control(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{},"cache_control":{"type":"ephemeral"}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{}"},"cache_control":{"type":"ephemeral"}}]}]"#
)]
#[case::tool_cache_control_stripped_for_gpt(
    "gpt-4o",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{},"cache_control":{"type":"ephemeral"}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{}"}}]}]"#
)]
#[case::non_string_fields(
    "claude-sonnet-5",
    r#"[{"role":"assistant","content":[{"type":"tool_use","id":5,"name":7,"input":{},"provider_specific_fields":{"signature":9}}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"","type":"function","function":{"name":"","arguments":"{}"}}]}]"#
)]
#[tokio::test]
async fn assistant_tool_signatures(
    #[case] model: &str,
    #[case] input: &str,
    #[case] expected: &str,
) {
    let (translated, probes) = run(input, model, false, &[]).await;
    assert_eq!(translated, json(expected));
    assert!(probes.is_empty());
}

#[rstest]
#[case::blocks_with_reasoning(
    r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"one","signature":"sig","cache_control":{"type":"ephemeral"}},{"type":"redacted_thinking","data":"hidden"}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":[{"type":"thinking","thinking":"one","signature":"sig"},{"type":"redacted_thinking","data":"hidden"}],"reasoning_content":"one"}]"#
)]
#[case::defaults_become_empty_strings(
    r#"[{"role":"assistant","content":[{"type":"thinking"},{"type":"redacted_thinking"}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":[{"type":"thinking","thinking":"","signature":""},{"type":"redacted_thinking","data":""}]}]"#
)]
#[case::two_readable_blocks_join_with_newline(
    r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"one","signature":"a"},{"type":"thinking","thinking":"two","signature":"b"}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":[{"type":"thinking","thinking":"one","signature":"a"},{"type":"thinking","thinking":"two","signature":"b"}],"reasoning_content":"one\ntwo"}]"#
)]
#[case::redacted_contributes_no_text(
    r#"[{"role":"assistant","content":[{"type":"redacted_thinking","data":"hidden"}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":[{"type":"redacted_thinking","data":"hidden"}]}]"#
)]
#[case::encrypted_reasoning_removed_with_its_text(
    r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"leak","signature":"litellm_encrypted_reasoning:abc"},{"type":"text","text":"kept"}]}]"#,
    r#"[{"role":"assistant","content":"kept","thinking_blocks":null}]"#
)]
#[case::encrypted_only_message_removed(
    r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"leak","signature":"litellm_encrypted_reasoning:abc"}]}]"#,
    "[]"
)]
#[case::non_string_fields(
    r#"[{"role":"assistant","content":[{"type":"thinking","thinking":5,"signature":6},{"type":"redacted_thinking","data":7}]}]"#,
    r#"[{"role":"assistant","content":null,"thinking_blocks":[{"type":"thinking","thinking":"","signature":""},{"type":"redacted_thinking","data":""}]}]"#
)]
#[tokio::test]
async fn assistant_thinking_history(#[case] input: &str, #[case] expected: &str) {
    let (translated, probes) = run(input, "claude-sonnet-5", false, &[]).await;
    assert_eq!(translated, json(expected));
    assert!(probes.is_empty());
}

const MIDDLE_SYSTEM: &str = r#"[{"role":"system","content":"lead"},{"role":"user","content":"ask"},{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{}}]},{"role":"system","content":[{"type":"text","text":"reminder"}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}]"#;

const MOVED: &str = r#"[{"role":"system","content":"lead"},{"role":"user","content":"ask"},{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{}"}}]},{"role":"tool","tool_call_id":"t","content":"ok"},{"role":"user","content":[{"type":"text","text":"Operator note (not from the user): the following was originally a mid-conversation system-role reminder."},{"type":"text","text":"reminder"}]}]"#;

const KEPT: &str = r#"[{"role":"system","content":"lead"},{"role":"user","content":"ask"},{"role":"assistant","content":null,"thinking_blocks":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"lookup","arguments":"{}"}}]},{"role":"system","content":[{"type":"text","text":"reminder"}]},{"role":"tool","tool_call_id":"t","content":"ok"}]"#;

#[rstest]
#[case::gpt_moves_after_tool_result("gpt-4o", MIDDLE_SYSTEM, false, &[], MOVED)]
#[case::preserved_keeps_position("gpt-4o", MIDDLE_SYSTEM, true, &[], KEPT)]
#[case::declared_capability_keeps_position("claude-sonnet-5", MIDDLE_SYSTEM, false, &["claude-sonnet-5"], KEPT)]
#[case::empty_system_content_removed(
    "claude-sonnet-5",
    r#"[{"role":"user","content":"ask"},{"role":"system","content":""},{"role":"system","content":[{"type":"image"}]},{"role":"user","content":"go"}]"#,
    true,
    &[],
    r#"[{"role":"user","content":"ask"},{"role":"user","content":"go"}]"#
)]
#[case::only_system_turns(
    "gpt-4o",
    r#"[{"role":"system","content":"a"},{"role":"system","content":"b"}]"#,
    false,
    &[],
    r#"[{"role":"system","content":"a"},{"role":"system","content":"b"}]"#
)]
#[tokio::test]
async fn midturn_system_capability(
    #[case] model: &str,
    #[case] input: &str,
    #[case] preserve: bool,
    #[case] supported: &[&'static str],
    #[case] expected: &str,
) {
    let (translated, _probes) = run(input, model, preserve, supported).await;
    assert_eq!(translated, json(expected));
}

#[rstest]
#[case::no_middle_system_no_probe(
    "gpt-4o",
    r#"[{"role":"system","content":"lead"},{"role":"user","content":"ask"}]"#,
    false,
    &[],
    vec![]
)]
#[case::middle_system_probes_the_target(
    "gpt-4o",
    MIDDLE_SYSTEM,
    false,
    &[],
    vec![("gpt-4o".to_owned(), Some("openai".to_owned()))]
)]
#[case::preserve_skips_the_probe("gpt-4o", MIDDLE_SYSTEM, true, &[], vec![])]
#[case::declared_model_probes_once(
    "claude-sonnet-5",
    MIDDLE_SYSTEM,
    false,
    &["claude-sonnet-5"],
    vec![("claude-sonnet-5".to_owned(), Some("anthropic".to_owned()))]
)]
#[tokio::test]
async fn midturn_system_capability_probes(
    #[case] model: &str,
    #[case] input: &str,
    #[case] preserve: bool,
    #[case] supported: &[&'static str],
    #[case] expected: Vec<Probe>,
) {
    let (_translated, probes) = run(input, model, preserve, supported).await;
    assert_eq!(probes, expected);
}
