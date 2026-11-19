#![cfg(test)]

use std::fs;

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde::Serialize;

use crate::api::api_boundaries::ApiLimits;

use super::{
    MessagesResponsesOutpuInput, decode_messages_responses_out, messages_responses_output,
};

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("valid fixture")
}

fn rendered<T>(value: &T) -> Json
where
    T: Serialize,
{
    let text = serde_json::to_string(value).expect("serializes");
    serde_json::from_str(&text).expect("valid json")
}

fn translated(source: &str) -> Json {
    let input: MessagesResponsesOutpuInput = serde_json::from_str(source).expect("valid input");
    let result = messages_responses_output(&input, &ApiLimits::default()).expect("translated");
    rendered(&result)
}

#[rstest]
#[case::thinking_null(
    r#"{"request":{"model":"claude","messages":[],"thinking":null}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::thinking_disabled(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"disabled"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::thinking_type_not_text(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":5}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::enabled_budget_low(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low"}}}"#
)]
#[case::enabled_budget_medium_at_threshold(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":2048}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"medium"}}}"#
)]
#[case::enabled_budget_high_at_threshold(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":4096}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"high"}}}"#
)]
#[case::enabled_budget_just_below_low(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1023}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"minimal"}}}"#
)]
#[case::enabled_budget_missing(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"minimal"}}}"#
)]
#[case::enabled_budget_float_truncates(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":2048.9}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"medium"}}}"#
)]
#[case::enabled_budget_text_is_zero(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":"5000"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"minimal"}}}"#
)]
#[case::adaptive_defaults_to_medium(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"adaptive"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"medium"}}}"#
)]
#[case::adaptive_output_config_effort(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"adaptive"},"output_config":{"effort":"high"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"high"}}}"#
)]
#[case::adaptive_empty_effort(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"adaptive"},"output_config":{"effort":""}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"medium"}}}"#
)]
#[case::adaptive_xhigh_effort(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"adaptive"},"output_config":{"effort":"xhigh"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"xhigh"}}}"#
)]
#[case::adaptive_max_effort(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"adaptive"},"output_config":{"effort":"max"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"max"}}}"#
)]
#[case::summary_passthrough(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500,"summary":"concise"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low","summary":"concise"}}}"#
)]
#[case::empty_summary_with_auto_summary_on(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500,"summary":""}},"settings":{"reasoning_auto_summary":true}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low","summary":"detailed"}}}"#
)]
#[case::auto_summary_on_no_summary(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500}},"settings":{"reasoning_auto_summary":true}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low","summary":"detailed"}}}"#
)]
#[case::auto_summary_off_no_summary(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500}},"settings":{"reasoning_auto_summary":false}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low"}}}"#
)]
#[case::configured_thresholds(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":250}},"settings":{"budget_thresholds":{"low":100,"medium":200,"high":300}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"medium"}}}"#
)]
fn thinking_reasoning_options(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[test]
fn thinking_unknown_effort_is_an_error() {
    let input: MessagesResponsesOutpuInput = serde_json::from_str(
        r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"adaptive"},"output_config":{"effort":"turbo"}}}"#,
    )
    .expect("valid input");
    let error = messages_responses_output(&input, &ApiLimits::default())
        .expect_err("the effort is not a declared level");
    assert_eq!(
        error.to_string(),
        "output_config.effort \"turbo\" is not a reasoning effort"
    );
}

#[rstest]
#[case::compact_with_value(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":150000}}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction","compact_threshold":150000}]}}"#
)]
#[case::compact_without_trigger(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112"}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction"}]}}"#
)]
#[case::trigger_without_value(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens"}}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction"}]}}"#
)]
#[case::trigger_value_null(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":null}}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction"}]}}"#
)]
#[case::trigger_not_an_object(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":5}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction"}]}}"#
)]
#[case::float_value_truncates(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":{"value":150000.7}}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction","compact_threshold":150000}]}}"#
)]
#[case::integer_text_value(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":{"value":" 150000 "}}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction","compact_threshold":150000}]}}"#
)]
#[case::unknown_edit_skipped(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"clear_tool_uses"}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::non_object_edit_skipped(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[5,{"type":"compact_20260112","trigger":{"value":10}}]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"context_management":[{"type":"compaction","compact_threshold":10}]}}"#
)]
#[case::edits_not_a_list(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":"x"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::edits_absent(
    r#"{"request":{"model":"claude","messages":[],"context_management":{}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::no_compaction_edits(
    r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[]}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
fn context_management_option(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[test]
fn context_threshold_not_integer_is_an_error() {
    let input: MessagesResponsesOutpuInput = serde_json::from_str(
        r#"{"request":{"model":"claude","messages":[],"context_management":{"edits":[{"type":"compact_20260112","trigger":{"value":"many"}}]}}}"#,
    )
    .expect("valid input");
    let error = messages_responses_output(&input, &ApiLimits::default())
        .expect_err("the threshold is not an integer");
    assert_eq!(
        error.to_string(),
        "context_management.edits[].trigger.value is not an integer"
    );
}

#[rstest]
#[case::any_becomes_required(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"type":"any"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":"required"}}"#
)]
#[case::tool_becomes_function(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"type":"tool","name":"lookup"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":{"type":"function","name":"lookup"}}}"#
)]
#[case::tool_without_name(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"type":"tool"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":{"type":"function","name":""}}}"#
)]
#[case::none_passes_through(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"type":"none"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":"none"}}"#
)]
#[case::auto_passes_through(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"type":"auto"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":"auto"}}"#
)]
#[case::missing_type_is_auto(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"name":"lookup"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":"auto"}}"#
)]
#[case::unknown_type_is_auto(
    r#"{"request":{"model":"claude","messages":[],"tool_choice":{"type":"banana"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tool_choice":"auto"}}"#
)]
fn tool_choice_required(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[rstest]
#[case::function_tool(
    r#"{"request":{"model":"claude","messages":[],"tools":[{"name":"lookup","description":"find","input_schema":{"type":"object"},"strict":true}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tools":[{"type":"function","name":"lookup","strict":true,"description":"find","parameters":{"type":"object"}}]}}"#
)]
#[case::tool_without_type_or_name(
    r#"{"request":{"model":"claude","messages":[],"tools":[{}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tools":[{"type":"function","name":"","strict":false}]}}"#
)]
#[case::web_search_type(
    r#"{"request":{"model":"claude","messages":[],"tools":[{"type":"web_search_20250305","name":"web_search"}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tools":[{"type":"web_search_preview"}]}}"#
)]
#[case::web_search_name_only(
    r#"{"request":{"model":"claude","messages":[],"tools":[{"type":"custom","name":"web_search"}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tools":[{"type":"web_search_preview"}]}}"#
)]
#[case::strict_null_defaults_false(
    r#"{"request":{"model":"claude","messages":[],"tools":[{"name":"lookup","strict":null}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"tools":[{"type":"function","name":"lookup","strict":false}]}}"#
)]
#[case::empty_tools_omitted(
    r#"{"request":{"model":"claude","messages":[],"tools":[]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::null_tools_omitted(
    r#"{"request":{"model":"claude","messages":[],"tools":null}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
fn tools_translation(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[rstest]
#[case::system_string_instructions(
    r#"{"request":{"model":"claude","messages":[],"system":"be brief"}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"instructions":"be brief"}}"#
)]
#[case::empty_system_string_omitted(
    r#"{"request":{"model":"claude","messages":[],"system":""}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::system_blocks_joined(
    r#"{"request":{"model":"claude","messages":[],"system":[{"type":"text","text":"a"},{"type":"text","text":""},{"type":"text","text":"b"},{"type":"image","text":"ignored"}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"instructions":"a\nb"}}"#
)]
#[case::system_blocks_without_text(
    r#"{"request":{"model":"claude","messages":[],"system":[{"type":"image"}]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"instructions":""}}"#
)]
#[case::empty_system_blocks_omitted(
    r#"{"request":{"model":"claude","messages":[],"system":[]}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::developer_empty_text_dropped(
    r#"{"request":{"model":"claude","messages":[],"system":[{"type":"text","text":"","prompt_cache_breakpoint":{"mode":"explicit"}},{"type":"text","text":"kept"}]}}"#,
    r#"{"request":{"model":"claude","input":[{"type":"message","role":"developer","content":[{"type":"input_text","text":"kept"}]}],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::developer_parts_replace_instructions(
    r#"{"request":{"model":"claude","messages":[{"role":"user","content":"hi"}],"system":[{"type":"text","text":"be brief","prompt_cache_breakpoint":{"mode":"explicit"}},{"type":"text","text":"second"}]}}"#,
    r#"{"request":{"model":"claude","input":[{"type":"message","role":"developer","content":[{"type":"input_text","text":"be brief","prompt_cache_breakpoint":{"mode":"explicit"}},{"type":"input_text","text":"second"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::metadata_user_and_session_key(
    r#"{"request":{"model":"claude","messages":[],"metadata":{"user_id":"{\"session_id\":\"sess-1\"}"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"user":"{\"session_id\":\"sess-1\"}","prompt_cache_key":"sess-1"}}"#
)]
#[case::metadata_plain_user_has_no_session_key(
    r#"{"request":{"model":"claude","messages":[],"metadata":{"user_id":"user-123"}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"user":"user-123"}}"#
)]
#[case::pass_through_sampling(
    r#"{"request":{"model":"claude","messages":[],"max_tokens":512,"temperature":0.5,"top_p":0.9}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"max_output_tokens":512,"temperature":0.5,"top_p":0.9}}"#
)]
#[case::zero_max_tokens_omitted(
    r#"{"request":{"model":"claude","messages":[],"max_tokens":0}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"]}}"#
)]
#[case::structured_output_format(
    r#"{"request":{"model":"claude","messages":[],"output_format":{"type":"json_schema","schema":{"type":"object"},"strict":true}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"text":{"format":{"type":"json_schema","name":"structured_output","schema":{"type":"object"},"strict":true}}}}"#
)]
#[case::structured_output_config_format(
    r#"{"request":{"model":"claude","messages":[],"output_config":{"format":{"type":"json_schema","schema":{"type":"object"}}}}}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"text":{"format":{"type":"json_schema","name":"structured_output","schema":{"type":"object"},"strict":false}}}}"#
)]
fn request_options(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[rstest]
#[case::text_then_function_call(
    r#"{"response":{"id":"resp_1","created_at":1,"model":"gpt-5","status":"completed","output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"hi","annotations":[]}]},{"type":"function_call","arguments":"{\"q\":\"x\"}","call_id":"call_1","name":"lookup","id":"fc_1"}]}}"#,
    r#"{"response":{"id":"resp_1","type":"message","role":"assistant","model":"gpt-5","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"call_1","name":"lookup","input":{"q":"x"}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::function_call_id_fallback(
    r#"{"response":{"id":"resp_2","output":[{"type":"function_call","arguments":"{}","call_id":"","name":"lookup","id":"fc_9"}]}}"#,
    r#"{"response":{"id":"resp_2","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"fc_9","name":"lookup","input":{}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::function_call_without_ids(
    r#"{"response":{"id":"resp_3","output":[{"type":"function_call","arguments":"{}","call_id":"","name":"lookup","id":null}]}}"#,
    r#"{"response":{"id":"resp_3","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"","name":"lookup","input":{}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::invalid_arguments_become_empty(
    r#"{"response":{"id":"resp_4","output":[{"type":"function_call","arguments":"{oops","call_id":"call_4","name":"lookup"}]}}"#,
    r#"{"response":{"id":"resp_4","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"call_4","name":"lookup","input":{}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::empty_arguments_become_empty(
    r#"{"response":{"id":"resp_5","output":[{"type":"function_call","arguments":"","call_id":"call_5","name":"lookup"}]}}"#,
    r#"{"response":{"id":"resp_5","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"call_5","name":"lookup","input":{}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::two_calls(
    r#"{"response":{"id":"resp_6","output":[{"type":"function_call","arguments":"{\"a\":1}","call_id":"call_a","name":"one"},{"type":"function_call","arguments":"{\"b\":2}","call_id":"call_b","name":"two"}]}}"#,
    r#"{"response":{"id":"resp_6","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"call_a","name":"one","input":{"a":1}},{"type":"tool_use","id":"call_b","name":"two","input":{"b":2}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::incomplete_is_max_tokens(
    r#"{"response":{"id":"resp_7","model":"gpt-5","status":"incomplete","output":[{"type":"message","id":"msg_7","role":"assistant","status":"incomplete","content":[{"type":"output_text","text":"cut","annotations":[]}]}]}}"#,
    r#"{"response":{"id":"resp_7","type":"message","role":"assistant","model":"gpt-5","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"cut"}],"stop_reason":"max_tokens","stop_details":null}}"#
)]
#[case::refusal_sets_stop_details(
    r#"{"response":{"id":"resp_8","model":"gpt-5","status":"completed","output":[{"type":"message","id":"msg_8","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"I cannot"}]}]}}"#,
    r#"{"response":{"id":"resp_8","type":"message","role":"assistant","model":"gpt-5","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"I cannot"}],"stop_reason":"refusal","stop_details":{"type":"refusal","category":null,"explanation":"I cannot"}}}"#
)]
#[case::empty_refusal_is_not_a_refusal(
    r#"{"response":{"id":"resp_9","model":"gpt-5","output":[{"type":"message","id":"msg_9","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":""}]}]}}"#,
    r#"{"response":{"id":"resp_9","type":"message","role":"assistant","model":"gpt-5","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":""}],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::usage_carried(
    r#"{"response":{"id":"resp_10","output":[],"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}"#,
    r#"{"response":{"id":"resp_10","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":20},"content":[],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::cached_usage_carried(
    r#"{"response":{"id":"resp_11","output":[],"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"input_tokens_details":{"cached_tokens":30}}}}"#,
    r#"{"response":{"id":"resp_11","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":70,"output_tokens":20,"cache_read_input_tokens":30},"content":[],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::empty_model_is_unknown(
    r#"{"response":{"id":"resp_12","model":"","output":[]}}"#,
    r#"{"response":{"id":"resp_12","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::raw_message_item(
    r#"{"response":{"id":"resp_13","output":[{"type":"message","content":[{"type":"output_text","text":"raw"}],"role":"assistant"}]}}"#,
    r#"{"response":{"id":"resp_13","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"raw"}],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::raw_function_call_item(
    r#"{"response":{"id":"resp_14","output":[{"type":"function_call","arguments":"{\"a\":1}","name":"lookup"}]}}"#,
    r#"{"response":{"id":"resp_14","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"","name":"lookup","input":{"a":1}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
#[case::raw_message_refusal_item(
    r#"{"response":{"id":"resp_15","output":[{"type":"message","content":[{"type":"refusal","refusal":"no"}],"role":"assistant"}]}}"#,
    r#"{"response":{"id":"resp_15","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"no"}],"stop_reason":"refusal","stop_details":{"type":"refusal","category":null,"explanation":"no"}}}"#
)]
#[case::raw_function_call_empty_call_id_falls_back(
    r#"{"response":{"id":"resp_16","output":[{"type":"function_call","arguments":5,"call_id":"","id":"fc_raw","name":"lookup"}]}}"#,
    r#"{"response":{"id":"resp_16","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"tool_use","id":"fc_raw","name":"lookup","input":{}}],"stop_reason":"tool_use","stop_details":null}}"#
)]
fn responses_function_output(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[test]
fn response_arguments_not_object_is_an_error() {
    let input: MessagesResponsesOutpuInput = serde_json::from_str(
        r#"{"response":{"id":"resp_1","output":[{"type":"function_call","arguments":"5","call_id":"call_1","name":"lookup"}]}}"#,
    )
    .expect("valid input");
    let error = messages_responses_output(&input, &ApiLimits::default())
        .expect_err("the arguments are not an object");
    assert_eq!(
        error.to_string(),
        "a Responses function call carries arguments that are not a JSON object"
    );
}

#[rstest]
#[case::typed_summary_and_encrypted(
    r#"{"response":{"id":"resp_1","output":[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"step"}],"encrypted_content":"ENC"}]}}"#,
    r#"{"response":{"id":"resp_1","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"thinking","thinking":"step","signature":"litellm_encrypted_reasoning:ENC"}],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::typed_encrypted_without_summary(
    r#"{"response":{"id":"resp_2","output":[{"type":"reasoning","id":"rs_2","summary":[],"encrypted_content":"ENC"}]}}"#,
    r#"{"response":{"id":"resp_2","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"redacted_thinking","data":"litellm_encrypted_reasoning:ENC"}],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::typed_summary_without_encryption(
    r#"{"response":{"id":"resp_3","output":[{"type":"reasoning","id":"rs_3","summary":[{"type":"summary_text","text":"step"}]}]}}"#,
    r#"{"response":{"id":"resp_3","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"thinking","thinking":"step","signature":null}],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::raw_reasoning_item(
    r#"{"response":{"id":"resp_4","output":[{"type":"reasoning","summary":[{"type":"summary_text","text":"step"}],"encrypted_content":"ENC"}]}}"#,
    r#"{"response":{"id":"resp_4","type":"message","role":"assistant","model":"unknown-model","stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"thinking","thinking":"step","signature":"litellm_encrypted_reasoning:ENC"}],"stop_reason":"end_turn","stop_details":null}}"#
)]
#[case::include_enabled_explicitly(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500}},"include_encrypted_reasoning":true}"#,
    r#"{"request":{"model":"claude","input":[],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low"}}}"#
)]
#[case::include_disabled(
    r#"{"request":{"model":"claude","messages":[],"thinking":{"type":"enabled","budget_tokens":1500}},"include_encrypted_reasoning":false}"#,
    r#"{"request":{"model":"claude","input":[],"reasoning":{"effort":"low"}}}"#
)]
fn encrypted_reasoning_response(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(translated(source), json(expected));
}

#[test]
fn replays_corpus() {
    let directory = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/messages_responses_output"
    );
    let outcomes: Vec<bool> = fs::read_dir(directory)
        .expect("the corpus exists")
        .map(|entry| {
            let path = entry.expect("the entry lists").path();
            let bytes = fs::read(path).expect("the seed reads");
            decode_messages_responses_out(&bytes, &ApiLimits::default()).is_ok()
        })
        .collect();
    assert!(outcomes.contains(&true));
    assert!(outcomes.contains(&false));
}
