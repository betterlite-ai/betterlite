use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Json;

use super::fixture::{Fixture, Session};
use crate::feature_flags::beta::{
    BetaRequest, EFFORT_BETA, FILE_CODE_EXECUTION_BETA, FILES_API_BETA, MCP_CLIENT_BETA,
    OUTPUT_CONFIG_BETA, THINKING_DISPLAY_BETA, TOOL_CHANGES_BETA, beta_list,
    computer_tool_beta_header, tool_search_beta_header, user_beta_headers,
};
use crate::feature_flags::billing::{billing_fields, billing_texts, is_one_shot_subagent_request};
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

const ONE_SHOT_SYSTEM_CASES: &[(&str, &str, bool)] = &[
    (
        r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#,
        r#""x-anthropic-billing-header: cc_is_subagent=true""#,
        false,
    ),
    (
        r#"[{"role":"assistant","content":"a"}]"#,
        r#""x-anthropic-billing-header: cc_is_subagent=true""#,
        false,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#"[{"type":"text","text":"   "}]"#,
        false,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#"[{"type":"image","source":{}}]"#,
        false,
    ),
    (r#"[{"role":"user","content":"hello"}]"#, "null", false),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#""x-anthropic-billing-header: cc_is_subagent=true""#,
        true,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#"[{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"}]"#,
        true,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#"[{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"},{"type":"text","text":"x-anthropic-billing-header: other=1"}]"#,
        true,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#"[{"type":"text","text":"   "},{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"}]"#,
        true,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#""  x-anthropic-billing-header: cc_is_subagent=true  ""#,
        true,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#"[{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"},{"type":"text","text":"x-anthropic-billing-header: cc_is_subagent=true"}]"#,
        false,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#""x-anthropic-billing-header: cc_is_subagent=false""#,
        false,
    ),
    (
        r#"[{"role":"user","content":"hello"}]"#,
        r#""x-anthropic-billing-header: cc_is_subagent=True""#,
        false,
    ),
];

fn one_shot_system_cases() {
    for (messages, system, expected) in ONE_SHOT_SYSTEM_CASES {
        assert_eq!(
            is_one_shot_subagent_request(
                &values(messages),
                &single(system),
                None,
                Some("claude-cli/1")
            ),
            *expected,
            "messages {messages} system {system}"
        );
    }
}

fn billing_text_cases() {
    assert_eq!(billing_texts(&single("[]")), None);
    assert_eq!(
        billing_texts(&single(r#"[{"type":"text","text":"   "}]"#)),
        None,
        "a whitespace-only text is not a billing text"
    );
    assert_eq!(
        billing_texts(&single(
            r#"[{"type":"text","text":"a"},{"type":"text","text":" "}]"#
        )),
        Some(vec!["a".to_owned()])
    );
}

pub(crate) fn one_shot_subagent_billing() {
    one_shot_identity_cases();
    one_shot_system_cases();
    billing_text_cases();
}

fn billing_field_parsed_cases() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "x-anthropic-billing-header: cc_is_subagent=true",
            &[("cc_is_subagent", "true")],
        ),
        (
            " \t x-anthropic-billing-header: a=1 ; b = 2 \n",
            &[("a", "1"), ("b", "2")],
        ),
        ("x-anthropic-billing-header: ;;a=1;", &[("a", "1")]),
        ("x-anthropic-billing-header:a=1", &[("a", "1")]),
        ("x-anthropic-billing-header: a=b=c", &[("a", "b=c")]),
        (
            "x-anthropic-billing-header: a=1;cc_is_subagent=true",
            &[("a", "1"), ("cc_is_subagent", "true")],
        ),
    ];
    for (source, expected) in cases {
        let fields: Vec<(String, String)> = expected
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        assert_eq!(billing_fields(source), Some(fields));
    }
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

pub(crate) fn malformed_billing_text() {
    billing_field_parsed_cases();
    billing_field_invalid();
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

const CACHE_CASES: &[(&str, Option<&str>, bool)] = &[
    ("claude-3-5-sonnet-20241022", Some("anthropic"), true),
    ("claude-3-5-sonnet-no-cache", Some("anthropic"), false),
    ("CLAUDE-3-5-SONNET-20241022", Some("anthropic"), true),
    ("gpt-4", Some("anthropic"), false),
    ("claude-3-5-sonnet-20241022", Some("openai"), false),
    ("claude-3-5-sonnet-20241022", Some(""), false),
    ("claude-unmapped", Some("anthropic"), false),
    ("anthropic/claude-3-5-sonnet-20241022", None, true),
    ("unknown/model", None, false),
    (
        "anthropic.claude-3-5-sonnet-20241022-v2:0",
        Some("bedrock"),
        true,
    ),
    ("claude-3-5-sonnet@20240620", Some("vertex_ai"), true),
    ("claude-3-5-sonnet", Some("azure_ai"), true),
    ("claude-3-5-sonnet", Some("azure"), false),
];

async fn cache_support_cases(fixture: &Fixture) {
    for (model, provider, expected) in CACHE_CASES {
        assert_cache(fixture, model, *provider, *expected).await;
    }
}

async fn assert_cache(fixture: &Fixture, model: &str, provider: Option<&str>, expected: bool) {
    assert_eq!(
        cache_case(fixture, model, provider).await,
        expected,
        "model {model} provider {provider:?}"
    );
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
    cache_support_cases(&fixture).await;
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
    let typeless_block = values(r#"[{"role":"user","content":[{"text":"hi"}]}]"#);
    assert!(matches!(
        is_file_id_used(&typeless_block),
        Err(FeatureFlagsError::Messages(_))
    ));
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

fn eager_flag_single_cases() {
    let cases: &[(&str, Option<bool>)] = &[
        (r#"{"eager_input_streaming":true}"#, Some(true)),
        (r#"{"eager_input_streaming":false}"#, Some(false)),
        (r#"{"eager_input_streaming":null}"#, None),
        ("{}", None),
        (r#""plain""#, None),
        ("[]", None),
        (r#"{"function":{"eager_input_streaming":true}}"#, Some(true)),
        (
            r#"{"eager_input_streaming":false,"function":{"eager_input_streaming":true}}"#,
            Some(false),
        ),
        (
            r#"{"eager_input_streaming":true,"function":{"eager_input_streaming":false}}"#,
            Some(true),
        ),
        (r#"{"eager_input_streaming":true,"extra":5}"#, Some(true)),
    ];
    for (source, expected) in cases {
        assert_eq!(
            eager_input_streaming_flag(&single(source)).unwrap(),
            *expected
        );
    }
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
    eager_flag_single_cases();
    eager_flag_rejections();
    eager_used_cases();
    assert_eq!(
        FeatureFlagsError::EagerInputStreaming.status_code(),
        Some(400),
        "LiteLLM raises UnsupportedParamsError, a 400, for a malformed flag"
    );
    let decode = serde_json::from_str::<Json>("{").unwrap_err();
    assert_eq!(FeatureFlagsError::Messages(decode).status_code(), None);
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

const ALL_FLAG_BETAS: &[Feature] = &[
    Feature::FileId,
    Feature::McpServer,
    Feature::MidConversationOutputConfig,
    Feature::ThinkingDisplayUpdates,
    Feature::MidConversationToolChange,
];

struct BetaCase {
    model: &'static str,
    params: &'static str,
    computer_tool: Option<&'static str>,
    flags: &'static [Feature],
    prompt_caching: bool,
    expected: &'static [&'static str],
}

const BETA_CASES: &[BetaCase] = &[
    BetaCase {
        model: "claude-sonnet-4-5",
        params: "{}",
        computer_tool: None,
        flags: &[],
        prompt_caching: false,
        expected: &[],
    },
    BetaCase {
        model: "claude-opus-4-5",
        params: r#"{"reasoning_effort":"high"}"#,
        computer_tool: None,
        flags: &[],
        prompt_caching: false,
        expected: &[EFFORT_BETA],
    },
    BetaCase {
        model: "claude-opus-4-7",
        params: r#"{"reasoning_effort":"high"}"#,
        computer_tool: None,
        flags: &[],
        prompt_caching: false,
        expected: &[],
    },
    BetaCase {
        model: "claude-sonnet-4-5",
        params: r#"{"output_config":{"effort":"medium"}}"#,
        computer_tool: None,
        flags: &[],
        prompt_caching: false,
        expected: &[EFFORT_BETA],
    },
    BetaCase {
        model: "claude-sonnet-4-5",
        params: "{}",
        computer_tool: Some("computer_20250124"),
        flags: &[],
        prompt_caching: false,
        expected: &["computer-use-2025-01-24"],
    },
    BetaCase {
        model: "claude-sonnet-4-5",
        params: "{}",
        computer_tool: Some(""),
        flags: &[],
        prompt_caching: false,
        expected: &[],
    },
    BetaCase {
        model: "claude-sonnet-4-5",
        params: "{}",
        computer_tool: None,
        flags: ALL_FLAG_BETAS,
        prompt_caching: false,
        expected: &[
            FILES_API_BETA,
            FILE_CODE_EXECUTION_BETA,
            MCP_CLIENT_BETA,
            OUTPUT_CONFIG_BETA,
            THINKING_DISPLAY_BETA,
            TOOL_CHANGES_BETA,
        ],
    },
    BetaCase {
        model: "claude-sonnet-4-5",
        params: "{}",
        computer_tool: None,
        flags: &[],
        prompt_caching: true,
        expected: &[],
    },
];

async fn beta_cases(fixture: &Fixture) {
    for case in BETA_CASES {
        let params = params(case.params);
        let features = AnthropicFeatures {
            flags: case.flags.iter().copied().collect(),
            computer_tool: case.computer_tool.map(str::to_owned),
        };
        let request = beta_request(case.model, &params, &features, case.prompt_caching);
        assert_eq!(
            betas(fixture, &request).await,
            case.expected
                .iter()
                .copied()
                .collect::<BTreeSet<&'static str>>(),
            "model {} params {}",
            case.model,
            case.params
        );
    }
}

pub(crate) async fn common_beta_set() {
    let fixture = Fixture::new(BETA_MODELS);
    beta_cases(&fixture).await;
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
