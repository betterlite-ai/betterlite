use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::merge_messages::merge_scoped_messages;
use betterlite_guardrails::guardrail_message_scope::merge_tools::merge_returned_tools;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags, ScanScope, ScopeFlags, filter_messages_by_skip_flags,
    scoped_message_indices,
};
use betterlite_guardrails::guardrail_message_scope::tool_names::{
    anthropic_tool_names, openai_tool_name,
};
use betterlite_server::guardrail_message_scope::text_slot::{
    message_slot_texts, message_with_slot_texts,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Flags {
    skip_system: Option<bool>,
    skip_tool: Option<bool>,
    scan_only: Option<bool>,
    global_skip_system: bool,
    global_skip_tool: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Extractor {
    Openai,
    Anthropic,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Slots {
    message: Object,
    texts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    messages: Vec<GuardrailValue>,
    flags: Flags,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolNames {
    tool: GuardrailValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeTools {
    extractor: Extractor,
    request: Option<Vec<GuardrailValue>>,
    returned: Vec<GuardrailValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeMessages {
    full: Vec<GuardrailValue>,
    scoped_indices: Vec<usize>,
    returned: Vec<GuardrailValue>,
}

pub(super) fn slots(request: &Slots) -> Value {
    let slot_texts = message_slot_texts(&request.message);
    json!({
        "slot_count": slot_texts.len(),
        "slot_texts": slot_texts,
        "rewritten": message_with_slot_texts(&request.message, &request.texts),
    })
}

pub(super) fn scope(request: &Scope) -> Value {
    let flags = &request.flags;
    let guardrail = GuardrailSkipFlags {
        skip_system_message: flags.skip_system,
        skip_tool_message: flags.skip_tool,
        scan_only_tool_results: flags.scan_only,
    };
    let global = GlobalSkipFlags {
        skip_system_message: flags.global_skip_system,
        skip_tool_message: flags.global_skip_tool,
    };
    let resolved = ScopeFlags::resolve(&guardrail, &global);
    let (filtered, changed) = filter_messages_by_skip_flags(&request.messages, &resolved);
    json!({
        "skip_system": resolved.skip_system_message,
        "skip_tool": resolved.skip_tool_message,
        "scan_only": resolved.scan == ScanScope::ToolResultsOnly,
        "indices": scoped_message_indices(&request.messages, &resolved),
        "filtered": filtered,
        "changed": changed,
    })
}

pub(super) fn tool_names(request: &ToolNames) -> Value {
    let names = anthropic_tool_names(&request.tool);
    json!({
        "openai": openai_tool_name(&request.tool),
        "anthropic": names,
        "anthropic_first": names.first(),
    })
}

pub(super) fn merge_tools(request: &MergeTools) -> Value {
    let request_tools = request.request.as_deref().unwrap_or_default();
    let merged = match request.extractor {
        Extractor::Openai => merge_returned_tools(request_tools, &request.returned, |tool| {
            openai_tool_name(tool).map(str::to_owned)
        }),
        Extractor::Anthropic => merge_returned_tools(request_tools, &request.returned, |tool| {
            anthropic_tool_names(tool)
                .first()
                .map(|name| (*name).to_owned())
        }),
    };
    json!(merged)
}

pub(super) fn merge_messages(request: &MergeMessages) -> Value {
    json!(merge_scoped_messages(
        &request.full,
        &request.scoped_indices,
        &request.returned
    ))
}
