use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Json;

use super::fixture::{Fixture, Session};
use crate::feature_flags::beta::{
    BetaRequest, EFFORT_BETA, FILE_CODE_EXECUTION_BETA, FILES_API_BETA, MCP_CLIENT_BETA,
    OUTPUT_CONFIG_BETA, THINKING_DISPLAY_BETA, TOOL_CHANGES_BETA, beta_list,
    computer_tool_beta_header, tool_search_beta_header, user_beta_headers,
};
use crate::feature_flags::billing::{billing_fields, is_one_shot_subagent_request};
use crate::feature_flags::cache::supports_cache_control;
use crate::feature_flags::compaction::requires_compaction_beta;
use crate::feature_flags::messages::{
    has_output_config, has_thinking_display_updates, has_tool_change, is_cache_control_set,
    is_file_id_used, is_mcp_server_used, is_pdf_used,
};
use crate::feature_flags::tools::{
    eager_input_streaming_flag, is_code_execution_tool_used, is_computer_tool_used,
    is_container_with_skills_used, is_eager_input_streaming_used, is_native_web_search_tool,
    is_tool_search_used, is_web_search_tool_used, uses_input_examples, uses_programmatic_calls,
};
use crate::feature_flags::{AnthropicFeatures, Feature, FeatureFlagsError, FeatureParams};

const NO_TOOLS: &[Json] = &[];

const CACHE_MODELS: &str = r#"{
    "anthropic/claude-3-5-sonnet-20241022": {"litellm_provider": "anthropic", "supports_prompt_caching": true},
    "anthropic/claude-3-5-sonnet-no-cache": {"litellm_provider": "anthropic", "supports_prompt_caching": false},
    "anthropic/gpt-4": {"litellm_provider": "anthropic", "supports_prompt_caching": true},
    "bedrock/anthropic.claude-3-5-sonnet-20241022-v2:0": {"litellm_provider": "bedrock", "supports_prompt_caching": true},
    "vertex_ai/claude-3-5-sonnet@20240620": {"litellm_provider": "vertex_ai", "supports_prompt_caching": true},
    "azure_ai/claude-3-5-sonnet": {"litellm_provider": "azure_ai", "supports_prompt_caching": true}
}"#;

const BETA_MODELS: &str = r#"{
    "anthropic/claude-opus-4-7": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "anthropic/claude-opus-4-5": {"litellm_provider": "anthropic", "supports_adaptive_thinking": false},
    "anthropic/claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

fn values(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn single(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn params(source: &str) -> FeatureParams {
    serde_json::from_str(source).unwrap()
}

fn text_pair(key: &str, value: &str) -> (String, String) {
    (key.to_owned(), value.to_owned())
}

fn one_shot_identity_cases() {
    let messages = values(r#"[{"role":"user","content":"hello"}]"#);
    let billing = single(r#""x-anthropic-billing-header: cc_is_subagent=true""#);
    let empty = values("[]");
    let tools = values(r#"[{"type":"function","function":{"name":"x"}}]"#);
    assert!(is_one_shot_subagent_request(
        &messages,
        &billing,
        None,
        Some("claude-cli/1.2.3")
    ));
    assert!(is_one_shot_subagent_request(
        &messages,
        &billing,
        Some(&empty),
        Some("claude-code/0.1")
    ));
    assert!(!is_one_shot_subagent_request(
        &messages,
        &billing,
        None,
        Some("Claude-CLI/1.2.3")
    ));
    assert!(!is_one_shot_subagent_request(
        &messages, &billing, None, None
    ));
    assert!(!is_one_shot_subagent_request(
        &messages,
        &billing,
        None,
        Some("curl/8")
    ));
    assert!(!is_one_shot_subagent_request(
        &messages,
        &billing,
        Some(&tools),
        Some("claude-cli/1")
    ));
}

fn one_shot_shape_cases() {
    let billing = single(r#""x-anthropic-billing-header: cc_is_subagent=true""#);
    let two = values(r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#);
    assert!(!is_one_shot_subagent_request(
        &two,
        &billing,
        None,
        Some("claude-cli/1")
    ));
    let assistant = values(r#"[{"role":"assistant","content":"a"}]"#);
    assert!(!is_one_shot_subagent_request(
        &assistant,
        &billing,
        None,
        Some("claude-cli/1")
    ));
}

fn one_shot_billing_lists() {
    let messages = values(r#"[{"role":"user","content":"hello"}]"#);
    let blocks =
        single(r#"[{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"}]"#);
    assert!(is_one_shot_subagent_request(
        &messages,
        &blocks,
        None,
        Some("claude-cli/1")
    ));
    let mixed_texts = single(
        r#"[{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"},{"type":"text","text":"x-anthropic-billing-header: other=1"}]"#,
    );
    assert!(is_one_shot_subagent_request(
        &messages,
        &mixed_texts,
        None,
        Some("claude-cli/1")
    ));
    let blank_then_billing = single(
        r#"[{"type":"text","text":"   "},{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"}]"#,
    );
    assert!(is_one_shot_subagent_request(
        &messages,
        &blank_then_billing,
        None,
        Some("claude-cli/1")
    ));
}

fn one_shot_system_rejections() {
    let messages = values(r#"[{"role":"user","content":"hello"}]"#);
    let blank_only = single(r#"[{"type":"text","text":"   "}]"#);
    assert!(!is_one_shot_subagent_request(
        &messages,
        &blank_only,
        None,
        Some("claude-cli/1")
    ));
    let non_text = single(r#"[{"type":"image","source":{}}]"#);
    assert!(!is_one_shot_subagent_request(
        &messages,
        &non_text,
        None,
        Some("claude-cli/1")
    ));
    assert!(!is_one_shot_subagent_request(
        &messages,
        &single("null"),
        None,
        Some("claude-cli/1")
    ));
}

fn one_shot_billing_values() {
    let messages = values(r#"[{"role":"user","content":"hello"}]"#);
    let padded = single(r#""  x-anthropic-billing-header: cc_is_subagent=true  ""#);
    assert!(is_one_shot_subagent_request(
        &messages,
        &padded,
        None,
        Some("claude-cli/1")
    ));
    let duplicate = single(
        r#"[{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"},{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"}]"#,
    );
    assert!(!is_one_shot_subagent_request(
        &messages,
        &duplicate,
        None,
        Some("claude-cli/1")
    ));
    let false_value = single(r#""x-anthropic-billing-header: cc_is_subagent=false""#);
    assert!(!is_one_shot_subagent_request(
        &messages,
        &false_value,
        None,
        Some("claude-cli/1")
    ));
    let capital = single(r#""x-anthropic-billing-header: cc_is_subagent=True""#);
    assert!(!is_one_shot_subagent_request(
        &messages,
        &capital,
        None,
        Some("claude-cli/1")
    ));
}

pub(crate) fn one_shot_subagent_billing() {
    one_shot_identity_cases();
    one_shot_shape_cases();
    one_shot_system_rejections();
    one_shot_billing_lists();
    one_shot_billing_values();
}

fn billing_field_valid() {
    assert_eq!(
        billing_fields("x-anthropic-billing-header: cc_is_subagent=true"),
        Some(vec![text_pair("cc_is_subagent", "true")])
    );
    assert_eq!(
        billing_fields(" \t x-anthropic-billing-header: a=1 ; b = 2 \n"),
        Some(vec![text_pair("a", "1"), text_pair("b", "2")])
    );
    assert_eq!(
        billing_fields("x-anthropic-billing-header: ;;a=1;"),
        Some(vec![text_pair("a", "1")])
    );
    assert_eq!(
        billing_fields("x-anthropic-billing-header:a=1"),
        Some(vec![text_pair("a", "1")])
    );
}

fn billing_field_invalid() {
    assert_eq!(billing_fields("x-anthropic-billing-header: a=1\nb=2"), None);
    assert_eq!(billing_fields("x-anthropic-billing-header: a=1\rb=2"), None);
    assert_eq!(billing_fields("a=1"), None);
    assert_eq!(billing_fields("x-anthropic-billing-header:"), None);
    assert_eq!(billing_fields("x-anthropic-billing-header: a"), None);
    assert_eq!(billing_fields("x-anthropic-billing-header: =1"), None);
    assert_eq!(billing_fields("x-anthropic-billing-header: a="), None);
    assert_eq!(billing_fields("x-anthropic-billing-header: a=  "), None);
    assert_eq!(billing_fields(""), None);
}

fn billing_field_splitting() {
    assert_eq!(
        billing_fields("x-anthropic-billing-header: a=b=c"),
        Some(vec![text_pair("a", "b=c")])
    );
    assert_eq!(
        billing_fields("x-anthropic-billing-header: a=1;cc_is_subagent=true"),
        Some(vec![
            text_pair("a", "1"),
            text_pair("cc_is_subagent", "true")
        ])
    );
}

pub(crate) fn malformed_billing_text() {
    billing_field_valid();
    billing_field_invalid();
    billing_field_splitting();
}

fn compaction_option_cases() {
    let compaction = params(r#"{"compaction": false}"#);
    assert!(requires_compaction_beta(
        "anthropic",
        Some(&compaction),
        NO_TOOLS
    ));
    let null_option = params(r#"{"compaction": null}"#);
    assert!(!requires_compaction_beta(
        "anthropic",
        Some(&null_option),
        NO_TOOLS
    ));
    let empty = params("{}");
    assert!(!requires_compaction_beta(
        "anthropic",
        Some(&empty),
        NO_TOOLS
    ));
    assert!(!requires_compaction_beta("anthropic", None, NO_TOOLS));
}

fn compaction_block_cases() {
    let signed = values(r#"[{"role":"user","content":[{"type":"compaction","signature":"sig"}]}]"#);
    assert!(requires_compaction_beta("anthropic", None, &signed));
    let empty_signature =
        values(r#"[{"role":"user","content":[{"type":"compaction","signature":""}]}]"#);
    assert!(!requires_compaction_beta(
        "anthropic",
        None,
        &empty_signature
    ));
    let numeric_signature = values(r#"[{"content":[{"type":"compaction","signature":5}]}]"#);
    assert!(!requires_compaction_beta(
        "anthropic",
        None,
        &numeric_signature
    ));
    let missing_signature = values(r#"[{"content":[{"type":"compaction"}]}]"#);
    assert!(!requires_compaction_beta(
        "anthropic",
        None,
        &missing_signature
    ));
    let string_content = values(r#"[{"content":"compaction"}]"#);
    assert!(!requires_compaction_beta(
        "anthropic",
        None,
        &string_content
    ));
    let mixed =
        values(r#"[{"content":["block",{"type":"text"},{"type":"compaction","signature":"s"}]}]"#);
    assert!(requires_compaction_beta("anthropic", None, &mixed));
    let non_object = values(r#"[7,{"content":[]}]"#);
    assert!(!requires_compaction_beta("anthropic", None, &non_object));
}

fn compaction_provider_cases() {
    let compaction = params(r#"{"compaction": true}"#);
    assert!(!requires_compaction_beta(
        "bedrock",
        Some(&compaction),
        NO_TOOLS
    ));
    assert!(!requires_compaction_beta(
        "vertex_ai",
        Some(&compaction),
        NO_TOOLS
    ));
}

pub(crate) fn native_compaction_signature() {
    compaction_option_cases();
    compaction_block_cases();
    compaction_provider_cases();
}

async fn cache_case(fixture: &Fixture, model: &str, provider: Option<&str>) -> bool {
    let mut session = Session::new(fixture);
    let mut environment = session.environment();
    supports_cache_control(model, provider, &mut environment).await
}

async fn cache_provider_cases(fixture: &Fixture) {
    let model = "claude-3-5-sonnet-20241022";
    assert!(cache_case(fixture, model, Some("anthropic")).await);
    assert!(
        !cache_case(fixture, "claude-3-5-sonnet-no-cache", Some("anthropic")).await,
        "an explicit false declaration wins"
    );
    assert!(
        cache_case(fixture, "CLAUDE-3-5-SONNET-20241022", Some("anthropic")).await,
        "the model name is matched case-insensitively"
    );
    assert!(!cache_case(fixture, "gpt-4", Some("anthropic")).await);
    assert!(!cache_case(fixture, model, Some("openai")).await);
    assert!(!cache_case(fixture, model, Some("")).await);
    assert!(!cache_case(fixture, "claude-unmapped", Some("anthropic")).await);
}

async fn cache_transport_cases(fixture: &Fixture) {
    assert!(cache_case(fixture, "anthropic/claude-3-5-sonnet-20241022", None).await);
    assert!(!cache_case(fixture, "unknown/model", None).await);
    assert!(
        cache_case(
            fixture,
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            Some("bedrock")
        )
        .await
    );
    assert!(cache_case(fixture, "claude-3-5-sonnet@20240620", Some("vertex_ai")).await);
    assert!(cache_case(fixture, "claude-3-5-sonnet", Some("azure_ai")).await);
    assert!(!cache_case(fixture, "claude-3-5-sonnet", Some("azure")).await);
}

fn cache_control_presence() {
    let message_control =
        values(r#"[{"role":"user","content":"hi","cache_control":{"type":"ephemeral"}}]"#);
    assert!(is_cache_control_set(&message_control));
    let null_control = values(r#"[{"role":"user","content":"hi","cache_control":null}]"#);
    assert!(!is_cache_control_set(&null_control));
    let block_control =
        values(r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":null}]}]"#);
    assert!(
        is_cache_control_set(&block_control),
        "a block's cache_control key counts even when its value is null"
    );
    let no_control = values(r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#);
    assert!(!is_cache_control_set(&no_control));
    let string_body = values(r#"[{"role":"user","content":"hi"}]"#);
    assert!(!is_cache_control_set(&string_body));
}

pub(crate) async fn cache_control_support() {
    let fixture = Fixture::new(CACHE_MODELS);
    cache_provider_cases(&fixture).await;
    cache_transport_cases(&fixture).await;
    cache_control_presence();
}

fn file_id_cases() {
    let file_message =
        values(r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-1"}}]}]"#);
    assert!(is_file_id_used(&file_message).unwrap());
    let text_message = values(r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#);
    assert!(!is_file_id_used(&text_message).unwrap());
    let assistant_file =
        values(r#"[{"role":"assistant","content":[{"type":"file","file":{"file_id":"file-2"}}]}]"#);
    assert!(!is_file_id_used(&assistant_file).unwrap());
    let empty_id = values(r#"[{"role":"user","content":[{"type":"file","file":{"file_id":""}}]}]"#);
    assert!(!is_file_id_used(&empty_id).unwrap());
    let string_content = values(r#"[{"role":"user","content":"file-1"}]"#);
    assert!(!is_file_id_used(&string_content).unwrap());
    let missing_file = values(r#"[{"role":"user","content":[{"type":"file"}]}]"#);
    assert!(!is_file_id_used(&missing_file).unwrap());
}

fn pdf_cases() {
    let file_message =
        values(r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-1"}}]}]"#);
    assert!(is_pdf_used(&file_message));
    let text_message = values(r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#);
    assert!(!is_pdf_used(&text_message));
    let image = values(r#"[{"role":"user","content":[{"type":"image","source":{}}]}]"#);
    assert!(is_pdf_used(&image));
    let tool_block =
        values(r#"[{"content":[{"type":"tool_use","id":"t","name":"n","input":{}}]}]"#);
    assert!(is_pdf_used(&tool_block));
    let null_type = values(r#"[{"content":[{"type":null}]}]"#);
    assert!(is_pdf_used(&null_type));
    let missing_type = values(r#"[{"content":[{"text":"hi"}]}]"#);
    assert!(!is_pdf_used(&missing_type));
    let string_body = values(r#"[{"content":"plain text"}]"#);
    assert!(!is_pdf_used(&string_body));
}

pub(crate) fn file_and_pdf_flags() {
    file_id_cases();
    pdf_cases();
}

fn tool_search_cases() {
    let regex = values(r#"[{"type":"tool_search_tool_regex_20251119"}]"#);
    assert!(is_tool_search_used(Some(&regex)));
    let bm25 = values(r#"[{"type":"tool_search_tool_bm25_20251119"}]"#);
    assert!(is_tool_search_used(Some(&bm25)));
    let similar = values(r#"[{"type":"tool_search_tool_regex_20251120"}]"#);
    assert!(!is_tool_search_used(Some(&similar)));
    let empty = values("[]");
    assert!(!is_tool_search_used(Some(&empty)));
    assert!(!is_tool_search_used(None));
}

fn programmatic_cases() {
    let top_level = values(r#"[{"allowed_callers":["code_execution_20250825"]}]"#);
    assert!(uses_programmatic_calls(Some(&top_level)));
    let nested = values(
        r#"[{"type":"function","function":{"allowed_callers":["code_execution_20250825"]}}]"#,
    );
    assert!(uses_programmatic_calls(Some(&nested)));
    let other_caller = values(r#"[{"allowed_callers":["code_execution_20250101"]}]"#);
    assert!(!uses_programmatic_calls(Some(&other_caller)));
    let empty_callers = values(r#"[{"allowed_callers":[]}]"#);
    assert!(!uses_programmatic_calls(Some(&empty_callers)));
    let string_callers = values(r#"[{"allowed_callers":"code_execution_20250825"}]"#);
    assert!(!uses_programmatic_calls(Some(&string_callers)));
    assert!(!uses_programmatic_calls(None));
}

fn example_cases() {
    let examples = values(r#"[{"input_examples":[{"value":"one"}]}]"#);
    assert!(uses_input_examples(Some(&examples)));
    let nested_examples =
        values(r#"[{"type":"function","function":{"input_examples":[{"value":"one"}]}}]"#);
    assert!(uses_input_examples(Some(&nested_examples)));
    let empty_examples = values(r#"[{"input_examples":[]}]"#);
    assert!(!uses_input_examples(Some(&empty_examples)));
    let text_examples = values(r#"[{"input_examples":"one"}]"#);
    assert!(!uses_input_examples(Some(&text_examples)));
    assert!(!uses_input_examples(None));
}

pub(crate) fn tool_search_and_examples() {
    tool_search_cases();
    programmatic_cases();
    example_cases();
}

fn assert_eager_rejected(tool: &Json) {
    let rejected = matches!(
        eager_input_streaming_flag(tool),
        Err(FeatureFlagsError::EagerInputStreaming)
    );
    assert!(
        rejected,
        "a malformed eager_input_streaming must raise UnsupportedParams"
    );
}

fn eager_flag_values() {
    assert_eq!(
        eager_input_streaming_flag(&single(r#"{"eager_input_streaming":true}"#)).unwrap(),
        Some(true)
    );
    assert_eq!(
        eager_input_streaming_flag(&single(r#"{"eager_input_streaming":false}"#)).unwrap(),
        Some(false)
    );
    assert_eq!(
        eager_input_streaming_flag(&single(r#"{"eager_input_streaming":null}"#)).unwrap(),
        None
    );
    assert_eq!(eager_input_streaming_flag(&single("{}")).unwrap(), None);
    assert_eq!(
        eager_input_streaming_flag(&single(r#""plain""#)).unwrap(),
        None
    );
    assert_eq!(eager_input_streaming_flag(&single("[]")).unwrap(), None);
}

fn eager_flag_nesting() {
    assert_eq!(
        eager_input_streaming_flag(&single(r#"{"function":{"eager_input_streaming":true}}"#))
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        eager_input_streaming_flag(&single(
            r#"{"eager_input_streaming":false,"function":{"eager_input_streaming":true}}"#
        ))
        .unwrap(),
        Some(false)
    );
    assert_eq!(
        eager_input_streaming_flag(&single(
            r#"{"eager_input_streaming":true,"function":{"eager_input_streaming":false}}"#
        ))
        .unwrap(),
        Some(true)
    );
    assert_eq!(
        eager_input_streaming_flag(&single(r#"{"eager_input_streaming":true,"extra":5}"#)).unwrap(),
        Some(true)
    );
}

fn eager_flag_rejections() {
    assert_eager_rejected(&single(r#"{"eager_input_streaming":"yes"}"#));
    assert_eager_rejected(&single(r#"{"eager_input_streaming":1}"#));
    assert_eager_rejected(&single(r#"{"function":{"eager_input_streaming":0}}"#));
    assert_eager_rejected(&single(r#"{"function":"x"}"#));
    assert_eager_rejected(&single(r#"{"function":[]}"#));
}

fn eager_used_cases() {
    let true_first = values(r#"[{"eager_input_streaming":true},{"eager_input_streaming":"bad"}]"#);
    assert!(is_eager_input_streaming_used(Some(&true_first)).unwrap());
    let bad_first = values(r#"[{"eager_input_streaming":"bad"},{"eager_input_streaming":true}]"#);
    assert!(
        is_eager_input_streaming_used(Some(&bad_first)).is_err(),
        "the malformed tool before the first true raises"
    );
    let false_only = values(r#"[{"eager_input_streaming":false}]"#);
    assert!(!is_eager_input_streaming_used(Some(&false_only)).unwrap());
    let empty = values("[]");
    assert!(!is_eager_input_streaming_used(Some(&empty)).unwrap());
    assert!(!is_eager_input_streaming_used(None).unwrap());
}

pub(crate) fn eager_input_boolean() {
    eager_flag_values();
    eager_flag_nesting();
    eager_flag_rejections();
    eager_used_cases();
}

fn computer_tool_cases() {
    let computer = values(r#"[{"type":"computer_20250124"}]"#);
    assert_eq!(
        is_computer_tool_used(Some(&computer)),
        Some("computer_20250124".to_owned())
    );
    assert_eq!(
        computer_tool_beta_header("computer_20250124"),
        "computer-use-2025-01-24"
    );
    assert_eq!(
        computer_tool_beta_header("computer_20241022"),
        "computer-use-2024-10-22"
    );
    assert_eq!(
        computer_tool_beta_header("computer_20990101"),
        "computer-use-2024-10-22"
    );
    let first_wins = values(r#"[{"type":"computer_20241022"},{"type":"computer_20250124"}]"#);
    assert_eq!(
        is_computer_tool_used(Some(&first_wins)),
        Some("computer_20241022".to_owned())
    );
    let untyped = values(r#"[{"name":"bash"}]"#);
    assert_eq!(is_computer_tool_used(Some(&untyped)), None);
    assert_eq!(is_computer_tool_used(None), None);
}

fn web_search_type_cases() {
    let versioned = values(r#"[{"type":"web_search_20250305"}]"#);
    assert!(is_web_search_tool_used(Some(&versioned)));
    let bare = values(r#"[{"type":"web_search"}]"#);
    assert!(is_web_search_tool_used(Some(&bare)));
    let other_suffix = values(r#"[{"type":"web_searches"}]"#);
    assert!(is_web_search_tool_used(Some(&other_suffix)));
    let name_only = values(r#"[{"name":"web_search"}]"#);
    assert!(!is_web_search_tool_used(Some(&name_only)));
    let empty = values("[]");
    assert!(!is_web_search_tool_used(Some(&empty)));
    assert!(!is_web_search_tool_used(None));
}

pub(crate) fn computer_tool_beta() {
    computer_tool_cases();
    web_search_type_cases();
}

fn code_execution_cases() {
    let execution = values(r#"[{"type":"code_execution_20250825"}]"#);
    assert!(is_code_execution_tool_used(Some(&execution)));
    let older = values(r#"[{"type":"code_execution_20250101"}]"#);
    assert!(!is_code_execution_tool_used(Some(&older)));
    assert!(!is_code_execution_tool_used(None));
}

fn container_skills_cases() {
    let skills = params(r#"{"container":{"skills":[{"type":"custom"}]}}"#);
    assert!(is_container_with_skills_used(Some(&skills)));
    let empty_skills = params(r#"{"container":{"skills":[]}}"#);
    assert!(!is_container_with_skills_used(Some(&empty_skills)));
    let no_skills = params(r#"{"container":{}}"#);
    assert!(!is_container_with_skills_used(Some(&no_skills)));
    let null_container = params(r#"{"container":null}"#);
    assert!(!is_container_with_skills_used(Some(&null_container)));
    let text_container = params(r#"{"container":"skills"}"#);
    assert!(!is_container_with_skills_used(Some(&text_container)));
    let text_skills = params(r#"{"container":{"skills":"one"}}"#);
    assert!(!is_container_with_skills_used(Some(&text_skills)));
    assert!(!is_container_with_skills_used(None));
}

fn user_header_cases() {
    assert_eq!(user_beta_headers(None), None);
    assert_eq!(
        user_beta_headers(Some("a,b")),
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        user_beta_headers(Some(" a , ,b ")),
        Some(vec![" a ".to_owned(), " ".to_owned(), "b ".to_owned()])
    );
    assert_eq!(user_beta_headers(Some("")), Some(vec![String::new()]));
}

fn tool_search_header_cases() {
    assert_eq!(
        tool_search_beta_header("vertex_ai"),
        "tool-search-tool-2025-10-19"
    );
    assert_eq!(
        tool_search_beta_header("vertex_ai_beta"),
        "tool-search-tool-2025-10-19"
    );
    assert_eq!(
        tool_search_beta_header("bedrock"),
        "tool-search-tool-2025-10-19"
    );
    assert_eq!(
        tool_search_beta_header("anthropic"),
        "advanced-tool-use-2025-11-20"
    );
    assert_eq!(
        tool_search_beta_header("azure"),
        "advanced-tool-use-2025-11-20"
    );
    assert_eq!(
        tool_search_beta_header("azure_ai"),
        "advanced-tool-use-2025-11-20"
    );
    assert_eq!(
        tool_search_beta_header("unknown"),
        "advanced-tool-use-2025-11-20"
    );
}

pub(crate) fn skills_code_execution_beta() {
    code_execution_cases();
    container_skills_cases();
    user_header_cases();
    tool_search_header_cases();
}

fn thinking_display_cases() {
    assert!(has_thinking_display_updates(Some(&single(
        r#"{"type":"enabled","display":"updates"}"#
    ))));
    assert!(has_thinking_display_updates(Some(&single(
        r#"{"type":"adaptive","display":"updates"}"#
    ))));
    assert!(!has_thinking_display_updates(Some(&single(
        r#"{"type":"disabled","display":"updates"}"#
    ))));
    assert!(!has_thinking_display_updates(Some(&single(
        r#"{"type":"enabled","display":"summarized"}"#
    ))));
    assert!(!has_thinking_display_updates(Some(&single("[]"))));
    assert!(!has_thinking_display_updates(None));
}

fn tool_change_cases() {
    let addition = values(
        r#"[{"role":"system","content":[{"type":"tool_addition","tool":{"type":"tool_reference"}}]}]"#,
    );
    assert!(has_tool_change(&addition));
    let removal = values(
        r#"[{"role":"system","content":[{"type":"tool_removal","tool":{"type":"tool_reference"}}]}]"#,
    );
    assert!(has_tool_change(&removal));
    let user_role = values(
        r#"[{"role":"user","content":[{"type":"tool_addition","tool":{"type":"tool_reference"}}]}]"#,
    );
    assert!(!has_tool_change(&user_role));
    let wrong_tool = values(
        r#"[{"role":"system","content":[{"type":"tool_addition","tool":{"type":"tool"}}]}]"#,
    );
    assert!(!has_tool_change(&wrong_tool));
    let missing_tool = values(r#"[{"role":"system","content":[{"type":"tool_addition"}]}]"#);
    assert!(!has_tool_change(&missing_tool));
    let string_content = values(r#"[{"role":"system","content":"tool_addition"}]"#);
    assert!(!has_tool_change(&string_content));
}

fn output_config_and_mcp_cases() {
    let with_config = values(r#"[{"role":"user","content":"hi","output_config":null}]"#);
    assert!(has_output_config(&with_config));
    let without = values(r#"[{"role":"user","content":"hi"}]"#);
    assert!(!has_output_config(&without));
    assert!(!is_mcp_server_used(None));
    let empty_servers = values("[]");
    assert!(!is_mcp_server_used(Some(&empty_servers)));
    let servers = values(r#"[{"type":"url","url":"https://mcp.example","name":"m"}]"#);
    assert!(is_mcp_server_used(Some(&servers)));
}

pub(crate) fn conversation_feature_flags() {
    thinking_display_cases();
    tool_change_cases();
    output_config_and_mcp_cases();
}

async fn betas(fixture: &Fixture, request: &BetaRequest<'_>) -> BTreeSet<&'static str> {
    let mut session = Session::new(fixture);
    let mut environment = session.environment();
    beta_list(request, &mut environment, &fixture.catalogs()).await
}

fn beta_request<'request>(
    model: &'request str,
    params: &'request FeatureParams,
    features: &'request AnthropicFeatures,
    prompt_caching_set: bool,
) -> BetaRequest<'request> {
    BetaRequest {
        model,
        custom_llm_provider: "anthropic",
        params: Some(params),
        features,
        prompt_caching_set,
    }
}

async fn effort_beta_cases(fixture: &Fixture) {
    let plain = params("{}");
    let features = AnthropicFeatures::default();
    let empty = beta_request("claude-sonnet-4-5", &plain, &features, false);
    assert!(betas(fixture, &empty).await.is_empty());
    let effort = params(r#"{"reasoning_effort":"high"}"#);
    let requested = beta_request("claude-opus-4-5", &effort, &features, false);
    assert_eq!(
        betas(fixture, &requested).await,
        BTreeSet::from([EFFORT_BETA])
    );
    let adaptive = beta_request("claude-opus-4-7", &effort, &features, false);
    assert!(
        betas(fixture, &adaptive).await.is_empty(),
        "an adaptive-thinking model needs no effort beta"
    );
    let output_config = params(r#"{"output_config":{"effort":"medium"}}"#);
    let configured = beta_request("claude-sonnet-4-5", &output_config, &features, false);
    assert_eq!(
        betas(fixture, &configured).await,
        BTreeSet::from([EFFORT_BETA])
    );
}

async fn feature_beta_cases(fixture: &Fixture) {
    let plain = params("{}");
    let computer = AnthropicFeatures {
        flags: BTreeSet::new(),
        computer_tool: Some("computer_20250124".to_owned()),
    };
    let computer_request = beta_request("claude-sonnet-4-5", &plain, &computer, false);
    assert_eq!(
        betas(fixture, &computer_request).await,
        BTreeSet::from(["computer-use-2025-01-24"])
    );
    let blank_computer = AnthropicFeatures {
        flags: BTreeSet::new(),
        computer_tool: Some(String::new()),
    };
    let blank_request = beta_request("claude-sonnet-4-5", &plain, &blank_computer, false);
    assert!(betas(fixture, &blank_request).await.is_empty());
}

async fn feature_flag_beta_cases(fixture: &Fixture) {
    let plain = params("{}");
    let all = AnthropicFeatures {
        flags: BTreeSet::from([
            Feature::FileId,
            Feature::McpServer,
            Feature::MidConversationOutputConfig,
            Feature::ThinkingDisplayUpdates,
            Feature::MidConversationToolChange,
        ]),
        computer_tool: None,
    };
    let all_request = beta_request("claude-sonnet-4-5", &plain, &all, false);
    assert_eq!(
        betas(fixture, &all_request).await,
        BTreeSet::from([
            FILES_API_BETA,
            FILE_CODE_EXECUTION_BETA,
            MCP_CLIENT_BETA,
            OUTPUT_CONFIG_BETA,
            THINKING_DISPLAY_BETA,
            TOOL_CHANGES_BETA,
        ])
    );
    let empty = AnthropicFeatures::default();
    let caching = beta_request("claude-sonnet-4-5", &plain, &empty, true);
    assert!(
        betas(fixture, &caching).await.is_empty(),
        "prompt_caching_set adds no beta header"
    );
}

pub(crate) async fn common_beta_set() {
    let fixture = Fixture::new(BETA_MODELS);
    effort_beta_cases(&fixture).await;
    feature_beta_cases(&fixture).await;
    feature_flag_beta_cases(&fixture).await;
}

pub(crate) fn native_web_search_type() {
    assert!(is_native_web_search_tool(&single(
        r#"{"type":"web_search_20250305","name":"web_search"}"#
    )));
    assert!(is_native_web_search_tool(&single(
        r#"{"type":"web_search_"}"#
    )));
    assert!(!is_native_web_search_tool(&single(
        r#"{"type":"web_search"}"#
    )));
    assert!(!is_native_web_search_tool(&single(
        r#"{"name":"web_search","type":"WebSearch"}"#
    )));
    assert!(!is_native_web_search_tool(&single(
        r#"{"name":"litellm_web_search"}"#
    )));
    assert!(!is_native_web_search_tool(&single(
        r#"{"type":"function","function":{"name":"litellm_web_search"}}"#
    )));
    assert!(!is_native_web_search_tool(&single(r#"{"type":5}"#)));
    assert!(!is_native_web_search_tool(&single(r#"{"type":null}"#)));
    assert!(!is_native_web_search_tool(&single("{}")));
    assert!(!is_native_web_search_tool(&single(
        r#""web_search_20250305""#
    )));
    assert!(is_native_web_search_tool(&single(
        r#"{"type":"web_search_unknown_suffix"}"#
    )));
}
