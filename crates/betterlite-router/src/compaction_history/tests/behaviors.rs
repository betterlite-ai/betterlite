use std::slice::from_ref;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::Tool;

use super::super::budget::budget;
use super::super::compactor::{
    CompactionCounter, CompactorConfig, ParentInput, ResponsesInput, compactor_input,
    compactor_model,
};
use super::super::contracts::{
    Candidate, Conversation, Deployment, History, InputBudget, ModelLimits, NativeCapability,
    OutputAllowance, ParentPayload, ParentTools, ResponsesPayload, SummaryRequest, SummaryTools,
};
use super::super::error::{CompactionError, ErrorKind};
use super::super::history::{portable_history, tool_ids};
use super::support::{
    CheckedConverter, FailingCounter, FixedCounter, RecordingCounter, candidate, deployment, items,
    json, messages, request,
};

const NO_TOOLS: &str = "[]";
const REASON_NO_COMPACTOR: &str = "No configured compactor supports native compaction with enough context and compatible defaults";
const REASON_NO_CLOSED_HISTORY: &str =
    "No closed older conversation is available without changing the latest request";
const REASON_NATIVE_REASONING: &str =
    "Native reasoning or compaction blocks require client-managed native compaction";
const ONE_EXCHANGE: &str = r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#;
const PAIR_EXCHANGE: &str = r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},{"type":"function_call","call_id":"call_1","name":"f","arguments":"{}"},{"type":"function_call_output","call_id":"call_1","output":"ok"},{"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#;
const TEXT_EXCHANGE: &str = r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]},{"type":"message","role":"user","content":[{"type":"text","text":"latest"}]}]"#;
const EMPTY_THINKING: &str = r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#;
const THINKING_BLOCKS: &str = r#"[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"x"}],"thinking_blocks":[{"type":"thinking","signature":"sig"}]}]"#;
const NULL_TYPE: &str = r#"[{"type":null,"role":"user","content":[{"type":"input_text","text":"first"}]},{"role":"user","content":[{"type":"input_text","text":"latest"}]}]"#;
const MISSING_TYPE: &str = r#"[{"role":"user","content":[{"type":"input_text","text":"first"}]},{"role":"user","content":[{"type":"input_text","text":"latest"}]}]"#;
const UNKNOWN_ITEM: &str = r#"[{"type":"web_search_call","id":"ws_1"}]"#;
const IMAGE_BLOCK: &str =
    r#"[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"x"}]}]"#;
const FUNCTION_TOOLS: &str = r#"[{"type":"function","name":"f","parameters":{"type":"object"}}]"#;
const OLDER_RESPONSES: &str = r#"[{"type":"message","role":"system","content":[{"type":"input_text","text":"sys"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#;
const OPEN_CALL: &str = r#"[{"role":"user","content":"first"},{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"user","content":"latest"}]"#;
const UNCLOSED_RESULT: &str = r#"[{"role":"user","content":"first"},{"role":"tool","tool_call_id":"call_1","content":"late"},{"role":"user","content":"latest"}]"#;
const EMPTY_ID: &str = r#"[{"role":"user","content":"first"},{"role":"assistant","tool_calls":[{"id":"","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"","content":"late"},{"role":"user","content":"latest"}]"#;
const DUPLICATE_CALLS: &str = r#"[{"role":"user","content":"first"},{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}},{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","content":"first result"},{"role":"tool","tool_call_id":"call_1","content":"second result"},{"role":"user","content":"latest"}]"#;
const EMPTY_PREFIX: &str = r#"[{"role":"user","content":"only"}]"#;
const LATE_SYSTEM: &str = r#"[{"role":"user","content":"first"},{"role":"system","content":"late instruction"},{"role":"user","content":"latest"}]"#;
const LATE_DEVELOPER: &str = r#"[{"role":"user","content":"first"},{"role":"developer","content":"late instruction"},{"role":"user","content":"latest"}]"#;
const CHAT_THINKING: &str = r#"[{"role":"user","content":"first"},{"role":"assistant","content":[{"type":"thinking","thinking":"hm","signature":"sig"}]},{"role":"user","content":"latest"}]"#;

type Case = fn();

const CASES: &[(&str, Case)] = &[
    ("keeps_a_closed_tool_pair", keeps_a_closed_tool_pair),
    ("keeps_block_tool_pairs", keeps_block_tool_pairs),
    (
        "keeps_responses_function_pairs",
        keeps_responses_function_pairs,
    ),
    (
        "treats_mid_conversation_instructions_as_unavailable",
        treats_mid_conversation_instructions_as_unavailable,
    ),
    (
        "treats_a_text_input_as_unavailable",
        treats_a_text_input_as_unavailable,
    ),
    (
        "treats_an_open_tool_call_as_unavailable",
        treats_an_open_tool_call_as_unavailable,
    ),
    (
        "treats_an_unclosed_tool_result_as_unavailable",
        treats_an_unclosed_tool_result_as_unavailable,
    ),
    (
        "treats_an_empty_tool_id_as_unavailable",
        treats_an_empty_tool_id_as_unavailable,
    ),
    (
        "treats_duplicate_tool_calls_as_unavailable",
        treats_duplicate_tool_calls_as_unavailable,
    ),
    (
        "treats_an_empty_prefix_as_unavailable",
        treats_an_empty_prefix_as_unavailable,
    ),
    (
        "treats_unknown_responses_items_as_unavailable",
        treats_unknown_responses_items_as_unavailable,
    ),
    (
        "treats_non_text_responses_blocks_as_unavailable",
        treats_non_text_responses_blocks_as_unavailable,
    ),
    (
        "treats_non_function_tools_as_unavailable",
        treats_non_function_tools_as_unavailable,
    ),
    (
        "treats_thinking_blocks_as_unavailable",
        treats_thinking_blocks_as_unavailable,
    ),
    (
        "treats_chat_thinking_blocks_as_unavailable",
        treats_chat_thinking_blocks_as_unavailable,
    ),
    ("keeps_empty_thinking_blocks", keeps_empty_thinking_blocks),
    (
        "treats_a_missing_responses_type_as_message",
        treats_a_missing_responses_type_as_message,
    ),
    (
        "treats_a_null_responses_type_as_unavailable",
        treats_a_null_responses_type_as_unavailable,
    ),
    (
        "budget_prefers_max_completion_tokens",
        budget_prefers_max_completion_tokens,
    ),
    (
        "budget_prefers_max_output_tokens",
        budget_prefers_max_output_tokens,
    ),
    (
        "budget_prefers_payload_max_tokens",
        budget_prefers_payload_max_tokens,
    ),
    (
        "budget_falls_back_to_model_output",
        budget_falls_back_to_model_output,
    ),
    (
        "budget_has_no_available_without_window",
        budget_has_no_available_without_window,
    ),
    (
        "budget_has_no_available_without_output",
        budget_has_no_available_without_output,
    ),
    (
        "budget_rejects_a_zero_output_allowance",
        budget_rejects_a_zero_output_allowance,
    ),
    (
        "budget_rejects_a_negative_output_allowance",
        budget_rejects_a_negative_output_allowance,
    ),
    (
        "compactor_input_keeps_tools_system_and_user",
        compactor_input_keeps_extra_keys,
    ),
    ("selects_the_configured_group", selects_the_configured_group),
    (
        "selects_the_first_eligible_group",
        selects_the_first_eligible_group,
    ),
    (
        "skips_a_group_without_a_provider",
        skips_a_group_without_a_provider,
    ),
    (
        "skips_a_group_that_lacks_support",
        skips_a_group_that_lacks_support,
    ),
    (
        "skips_a_group_with_incompatible_defaults",
        skips_a_group_with_incompatible_defaults,
    ),
    (
        "skips_a_group_that_cannot_hold_the_summary",
        skips_a_group_that_cannot_hold_the_summary,
    ),
    (
        "responses_compactor_input_uses_the_older_history",
        responses_compactor_input_uses_the_older_history,
    ),
    ("counts_the_summary_request", counts_the_summary_request),
    ("propagates_a_counter_failure", propagates_a_counter_failure),
    (
        "keeps_responses_function_tools",
        keeps_responses_function_tools,
    ),
    (
        "keeps_output_text_and_text_blocks",
        keeps_output_text_and_text_blocks,
    ),
    (
        "refuses_a_redacted_thinking_block",
        refuses_a_redacted_thinking_block,
    ),
    ("refuses_a_compaction_block", refuses_a_compaction_block),
    (
        "compactor_input_keeps_anthropic_tools",
        compactor_input_keeps_anthropic_tools,
    ),
    (
        "treats_a_developer_instruction_as_unavailable",
        treats_a_developer_instruction_as_unavailable,
    ),
];

pub(super) fn run(name: &str) {
    let Some((_, case)) = CASES.iter().find(|(case_name, _)| *case_name == name) else {
        panic!("unknown behavior case {name}");
    };
    (*case)();
}

fn history_error(conversation: &Conversation<'_>) -> CompactionError {
    portable_history(conversation, "parent-model").unwrap_err()
}

fn messages_error(raw: &str) -> CompactionError {
    let conversation = messages(raw);
    history_error(&Conversation::Messages(conversation.as_slice()))
}

fn responses_error(items_raw: &str, tools_raw: &str) -> CompactionError {
    responses_history(items_raw, tools_raw).unwrap_err()
}

fn responses_history(items_raw: &str, tools_raw: &str) -> Result<History, CompactionError> {
    let conversation = items(items_raw);
    let tools = items(tools_raw);
    portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
}

fn keeps_a_closed_tool_pair() {
    let conversation = messages(
        r#"[
            {"role":"system","content":"You are terse."},
            {"role":"developer","content":"Follow the schema."},
            {"role":"user","content":"first"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call_1","content":"42"},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(
        history,
        History {
            instruction_count: 2,
            prefix_count: 3,
            tail_count: 1,
            calls: vec!["call_1".to_owned()],
            results: vec!["call_1".to_owned()],
        }
    );
    assert_eq!(
        tool_ids(&conversation, "parent-model", false).unwrap(),
        vec!["call_1".to_owned()]
    );
    assert_eq!(
        tool_ids(&conversation, "parent-model", true).unwrap(),
        vec!["call_1".to_owned()]
    );
}

fn keeps_block_tool_pairs() {
    let conversation = messages(
        r#"[
            {"role":"user","content":[{"type":"text","text":"go"}]},
            {"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"f","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.instruction_count, 0);
    assert_eq!(history.prefix_count, 3);
    assert_eq!(history.tail_count, 1);
    assert_eq!(history.calls, vec!["toolu_1".to_owned()]);
    assert_eq!(history.results, vec!["toolu_1".to_owned()]);
}

fn keeps_responses_function_pairs() {
    let history = responses_history(PAIR_EXCHANGE, NO_TOOLS).unwrap();
    assert_eq!(history.prefix_count, 3);
    assert_eq!(history.tail_count, 1);
    assert_eq!(history.calls, vec!["call_1".to_owned()]);
    assert_eq!(history.results, vec!["call_1".to_owned()]);
}

fn keeps_empty_thinking_blocks() {
    let history = responses_history(EMPTY_THINKING, NO_TOOLS).unwrap();
    assert_eq!(history.prefix_count, 1);
    assert_eq!(history.tail_count, 1);
}

fn treats_a_missing_responses_type_as_message() {
    let history = responses_history(MISSING_TYPE, NO_TOOLS).unwrap();
    assert_eq!(history.prefix_count, 1);
}

fn treats_a_null_responses_type_as_unavailable() {
    let error = responses_error(NULL_TYPE, NO_TOOLS);
    assert_eq!(
        error.reason,
        "Opaque or provider-managed Responses items require client-managed native compaction"
    );
}

fn keeps_responses_function_tools() {
    let history = responses_history(ONE_EXCHANGE, FUNCTION_TOOLS).unwrap();
    assert_eq!(history.prefix_count, 1);
    assert_eq!(history.tail_count, 1);
}

fn keeps_output_text_and_text_blocks() {
    let history = responses_history(TEXT_EXCHANGE, NO_TOOLS).unwrap();
    assert_eq!(history.prefix_count, 2);
    assert_eq!(history.tail_count, 1);
}

fn treats_mid_conversation_instructions_as_unavailable() {
    let error = messages_error(LATE_SYSTEM);
    assert_eq!(
        error.reason,
        "Mid-conversation instructions cannot be compacted"
    );
}

fn treats_a_developer_instruction_as_unavailable() {
    let error = messages_error(LATE_DEVELOPER);
    assert_eq!(
        error.reason,
        "Mid-conversation instructions cannot be compacted"
    );
}

fn treats_a_text_input_as_unavailable() {
    let error = history_error(&Conversation::Text("hello"));
    assert_eq!(
        error.reason,
        "A single user input cannot be compacted without changing the latest request"
    );
}

fn treats_an_open_tool_call_as_unavailable() {
    let error = messages_error(OPEN_CALL);
    assert_eq!(error.reason, REASON_NO_CLOSED_HISTORY);
}

fn treats_an_unclosed_tool_result_as_unavailable() {
    let error = messages_error(UNCLOSED_RESULT);
    assert_eq!(error.reason, REASON_NO_CLOSED_HISTORY);
}

fn treats_an_empty_tool_id_as_unavailable() {
    let error = messages_error(EMPTY_ID);
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.reason, REASON_NO_CLOSED_HISTORY);
}

fn treats_duplicate_tool_calls_as_unavailable() {
    let error = messages_error(DUPLICATE_CALLS);
    assert_eq!(error.reason, REASON_NO_CLOSED_HISTORY);
}

fn treats_an_empty_prefix_as_unavailable() {
    let error = messages_error(EMPTY_PREFIX);
    assert_eq!(error.reason, REASON_NO_CLOSED_HISTORY);
}

fn treats_unknown_responses_items_as_unavailable() {
    let error = responses_error(UNKNOWN_ITEM, NO_TOOLS);
    assert_eq!(
        error.reason,
        "Opaque or provider-managed Responses items require client-managed native compaction"
    );
}

fn treats_non_text_responses_blocks_as_unavailable() {
    let error = responses_error(IMAGE_BLOCK, NO_TOOLS);
    assert_eq!(
        error.reason,
        "Only text history and ordinary function tools support portable Responses compaction"
    );
}

fn treats_non_function_tools_as_unavailable() {
    let error = responses_error(ONE_EXCHANGE, r#"[{"type":"web_search"}]"#);
    assert_eq!(
        error.reason,
        "Only text history and ordinary function tools support portable Responses compaction"
    );
}

fn treats_thinking_blocks_as_unavailable() {
    let error = responses_error(THINKING_BLOCKS, NO_TOOLS);
    assert_eq!(error.reason, REASON_NATIVE_REASONING);
}

fn treats_chat_thinking_blocks_as_unavailable() {
    let error = messages_error(CHAT_THINKING);
    assert_eq!(error.reason, REASON_NATIVE_REASONING);
}

fn allowance(
    max_completion_tokens: Option<i64>,
    max_output_tokens: Option<i64>,
    max_tokens: Option<i64>,
) -> OutputAllowance {
    OutputAllowance {
        max_completion_tokens,
        max_output_tokens,
        max_tokens,
    }
}

fn budget_case(allowance: OutputAllowance, model_output: Option<i64>) {
    let limits = ModelLimits {
        max_input_tokens: Some(1000),
        max_output_tokens: model_output,
    };
    let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
    assert_eq!(computed.window, Some(1000));
    let output = allowance
        .max_completion_tokens
        .or(allowance.max_output_tokens)
        .or(allowance.max_tokens)
        .or(model_output)
        .unwrap_or_default();
    assert_eq!(computed.available, Some(900_i64.saturating_sub(output)));
}

fn budget_error_case(output: Option<i64>, reason: &str) {
    let limits = ModelLimits {
        max_input_tokens: Some(1000),
        max_output_tokens: Some(11),
    };
    let error = budget("parent-model", limits, allowance(output, None, None), 0.9).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert_eq!(error.model, "parent-model");
    assert_eq!(error.reason, reason);
}

fn no_available(
    window: Option<i64>,
    model_output: Option<i64>,
    max_completion_tokens: Option<i64>,
) -> InputBudget {
    let limits = ModelLimits {
        max_input_tokens: window,
        max_output_tokens: model_output,
    };
    budget(
        "parent-model",
        limits,
        allowance(max_completion_tokens, None, None),
        0.9,
    )
    .unwrap()
}

fn budget_prefers_max_completion_tokens() {
    budget_case(allowance(Some(7), Some(9), Some(11)), Some(50));
}

fn budget_prefers_max_output_tokens() {
    budget_case(allowance(None, Some(9), Some(11)), Some(50));
}

fn budget_prefers_payload_max_tokens() {
    budget_case(allowance(None, None, Some(11)), Some(50));
}

fn budget_falls_back_to_model_output() {
    budget_case(allowance(None, None, None), Some(11));
}

fn budget_has_no_available_without_window() {
    let computed = no_available(None, Some(11), Some(7));
    assert_eq!(computed.window, None);
    assert_eq!(computed.available, None);
}

fn budget_has_no_available_without_output() {
    let computed = no_available(Some(1000), None, None);
    assert_eq!(computed.window, Some(1000));
    assert_eq!(computed.available, None);
}

fn budget_rejects_a_zero_output_allowance() {
    budget_error_case(Some(0), "The output allowance must be a positive integer");
}

fn budget_rejects_a_negative_output_allowance() {
    budget_error_case(Some(-3), "The output allowance must be a positive integer");
}

fn compactor_input_keeps_extra_keys() {
    let conversation = messages(
        r#"[{"role":"system","content":"sys"},
            {"role":"user","content":"first"},
            {"role":"assistant","content":"answer"},
            {"role":"user","content":"latest"}]"#,
    );
    let tools: Vec<Tool> =
        serde_json::from_str(r#"[{"type":"function","function":{"name":"f","arguments":"{}"}}]"#)
            .unwrap();
    let system = json("\"system prompt\"");
    let user = json("\"caller\"");
    let payload = ParentPayload {
        model: "parent-model",
        messages: conversation.as_slice(),
        tools: Some(ParentTools::Chat(&tools)),
        system: Some(&system),
        user: Some(&user),
    };
    let history = History {
        instruction_count: 1,
        prefix_count: 2,
        tail_count: 1,
        calls: Vec::new(),
        results: Vec::new(),
    };
    let summary = compactor_input(&ParentInput::Chat(&payload), &history, 4096).unwrap();
    assert_eq!(summary.model, "parent-model");
    assert_eq!(summary.max_tokens, 4096);
    assert_eq!(summary.messages.as_slice(), &conversation[..3]);
    assert_eq!(summary.tools, Some(SummaryTools::Chat(tools)));
    assert_eq!(summary.system, Some(system));
    assert_eq!(summary.user, Some(user));
}

fn compactor_config(model: Option<&str>) -> CompactorConfig<'_> {
    CompactorConfig {
        model,
        max_tokens: 512,
    }
}

fn select(
    candidates: &[Candidate],
    config_model: Option<&str>,
    counter: &dyn CompactionCounter,
) -> Result<String, CompactionError> {
    super::runtime()
        .block_on(compactor_model(
            &compactor_config(config_model),
            candidates,
            &request(512),
            counter,
        ))
        .map(|selection| selection.model)
}

fn capable_deployment() -> Deployment {
    deployment(true, NativeCapability::Supported, Some(200_000), Some(4096))
}

fn small_group(provider: bool, capability: NativeCapability, window: i64) -> Vec<Candidate> {
    vec![candidate(
        "small",
        vec![deployment(provider, capability, Some(window), Some(4096))],
    )]
}

fn supported_group() -> Vec<Candidate> {
    vec![candidate("large", vec![capable_deployment()])]
}

fn two_groups(first_provider: bool) -> Vec<Candidate> {
    vec![
        candidate(
            "small",
            vec![deployment(
                first_provider,
                NativeCapability::Supported,
                Some(200_000),
                Some(4096),
            )],
        ),
        candidate("large", vec![capable_deployment()]),
    ]
}

fn skip_case(candidates: &[Candidate]) {
    let error = select(candidates, None, &FixedCounter { value: 10 }).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
    assert_eq!(error.reason, REASON_NO_COMPACTOR);
}

fn skips_a_group_without_a_provider() {
    skip_case(&small_group(false, NativeCapability::Supported, 200_000));
}

fn skips_a_group_that_lacks_support() {
    skip_case(&small_group(true, NativeCapability::Unsupported, 200_000));
}

fn skips_a_group_with_incompatible_defaults() {
    skip_case(&small_group(
        true,
        NativeCapability::IncompatibleDefaults,
        200_000,
    ));
}

fn skips_a_group_that_cannot_hold_the_summary() {
    skip_case(&small_group(true, NativeCapability::Supported, 500));
}

fn selects_the_configured_group() {
    let selected = select(
        &two_groups(true),
        Some("large"),
        &FixedCounter { value: 10 },
    );
    assert_eq!(selected.unwrap(), "large");
}

fn selects_the_first_eligible_group() {
    let selected = select(&two_groups(false), None, &FixedCounter { value: 10 });
    assert_eq!(selected.unwrap(), "large");
}

struct OlderResponses {
    conversation: Vec<Json>,
    tools: Vec<Json>,
    request: Json,
    system: Json,
    user: Json,
}

fn older_responses() -> OlderResponses {
    OlderResponses {
        conversation: items(OLDER_RESPONSES),
        tools: items(r#"[{"type":"function","name":"f"}]"#),
        request: json(r#"{"model":"parent-model"}"#),
        system: json("\"instructions\""),
        user: json("\"caller\""),
    }
}

fn assert_older_summary(
    summary: &SummaryRequest,
    converter: &CheckedConverter,
    responses: &OlderResponses,
) {
    assert_eq!(summary.model, "parent-model");
    assert_eq!(
        summary.messages,
        messages(r#"[{"role":"user","content":"older"}]"#)
    );
    assert_eq!(summary.tools, Some(SummaryTools::Chat(Vec::new())));
    assert_eq!(
        (summary.system.as_ref(), summary.user.as_ref()),
        (Some(&responses.system), Some(&responses.user))
    );
    assert_eq!(summary.max_tokens, 4096);
    assert_eq!(converter.items.lock().unwrap().as_slice(), from_ref(&3));
    assert_eq!(
        converter.requests.lock().unwrap().as_slice(),
        from_ref(&responses.request)
    );
    assert_eq!(converter.tools.lock().unwrap().as_slice(), from_ref(&1));
}

fn responses_compactor_input_uses_the_older_history() {
    let responses = older_responses();
    let payload = ResponsesPayload {
        model: "parent-model",
        items: responses.conversation.as_slice(),
        tools: responses.tools.as_slice(),
        request: &responses.request,
        system: Some(&responses.system),
        user: Some(&responses.user),
    };
    let history = History {
        instruction_count: 1,
        prefix_count: 2,
        tail_count: 1,
        calls: Vec::new(),
        results: Vec::new(),
    };
    let converter = CheckedConverter {
        items: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        tools: Mutex::new(Vec::new()),
    };
    let summary = compactor_input(
        &ParentInput::Responses(&ResponsesInput {
            payload: &payload,
            converter: &converter,
        }),
        &history,
        4096,
    )
    .unwrap();
    assert_older_summary(&summary, &converter, &responses);
}

fn counts_the_summary_request() {
    let counter = RecordingCounter {
        seen: Mutex::new(Vec::new()),
    };
    let summary = request(512);
    let candidates = supported_group();
    let selected = super::runtime()
        .block_on(compactor_model(
            &compactor_config(None),
            &candidates,
            &summary,
            &counter,
        ))
        .unwrap();
    assert_eq!(selected.model, "large");
    assert_eq!(counter.seen.lock().unwrap().as_slice(), &[summary]);
}

fn propagates_a_counter_failure() {
    let error = select(&supported_group(), None, &FailingCounter).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert_eq!(error.reason, "the counter failed");
}

fn chat_reasoning_block_reason(block: &str) -> String {
    let raw = format!(
        r#"[{{"role":"user","content":"first"}},{{"role":"assistant","content":[{block}]}},{{"role":"user","content":"latest"}}]"#
    );
    let conversation = messages(&raw);
    history_error(&Conversation::Messages(conversation.as_slice())).reason
}

fn refuses_a_redacted_thinking_block() {
    assert_eq!(
        chat_reasoning_block_reason(r#"{"type":"redacted_thinking","data":"sig"}"#),
        REASON_NATIVE_REASONING
    );
}

fn refuses_a_compaction_block() {
    assert_eq!(
        chat_reasoning_block_reason(r#"{"type":"compaction","id":"cmp_1"}"#),
        REASON_NATIVE_REASONING
    );
}

fn compactor_input_keeps_anthropic_tools() {
    let conversation = messages(
        r#"[{"role":"user","content":"first"},
            {"role":"assistant","content":"answer"},
            {"role":"user","content":"latest"}]"#,
    );
    let tools = items(r#"[{"name":"lookup","input_schema":{"type":"object"}}]"#);
    let payload = ParentPayload {
        model: "parent-model",
        messages: conversation.as_slice(),
        tools: Some(ParentTools::Anthropic(tools.as_slice())),
        system: None,
        user: None,
    };
    let history = History {
        instruction_count: 0,
        prefix_count: 2,
        tail_count: 1,
        calls: Vec::new(),
        results: Vec::new(),
    };
    let summary = compactor_input(&ParentInput::Chat(&payload), &history, 4096).unwrap();
    assert_eq!(summary.tools, Some(SummaryTools::Anthropic(tools)));
}
