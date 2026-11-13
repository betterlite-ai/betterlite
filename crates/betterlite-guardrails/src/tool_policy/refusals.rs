use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::names::tool_call_names_from_messages;
use super::{POLICY_BLOCKED, POLICY_UNTRUSTED, ToolPolicyLookup};
use crate::guardrail_base::failure::GuardrailFailure;

const ERROR_TEXT: &str = "Violated tool policy";
const BLOCKED_STATUS: u16 = 400;

// LiteLLM writes these names into its messages through str() of a list of strings
fn quoted_names(names: &[String]) -> String {
    format!(
        "[{}]",
        names
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<String>>()
            .join(", ")
    )
}

fn names_value(names: &[String]) -> GuardrailValue {
    let listed: Vec<GuardrailValue> = names
        .iter()
        .map(|name| GuardrailValue::Text(name.clone()))
        .collect();
    GuardrailValue::Array(listed)
}

fn detail(entries: &[(&str, GuardrailValue)]) -> IndexMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[must_use]
pub(super) fn blocked_failure(
    policy_map: &BTreeMap<String, String>,
    tool_names: &[String],
) -> Option<GuardrailFailure> {
    let blocked: Vec<String> = tool_names
        .iter()
        .filter(|name| {
            policy_map
                .get(name.as_str())
                .is_some_and(|policy| policy.as_str() == POLICY_BLOCKED)
        })
        .cloned()
        .collect();
    if blocked.is_empty() {
        return None;
    }
    tracing::warn!(
        "ToolPolicyGuardrail: blocking tool(s) {} (input_policy=blocked)",
        quoted_names(&blocked)
    );
    Some(GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail: detail(&[
            ("error", GuardrailValue::Text(ERROR_TEXT.to_owned())),
            ("blocked_tools", names_value(&blocked)),
            (
                "message",
                GuardrailValue::Text(format!(
                    "Tool(s) {} are blocked by policy.",
                    quoted_names(&blocked)
                )),
            ),
        ]),
    })
}

fn untrusted_sources(messages: &[GuardrailValue], lookup: &dyn ToolPolicyLookup) -> Vec<String> {
    let call_names = tool_call_names_from_messages(messages);
    let mut sources: Vec<String> = Vec::new();
    for message in messages {
        let GuardrailValue::Object(fields) = message else {
            continue;
        };
        if !matches!(fields.get("role"), Some(GuardrailValue::Text(role)) if role == "tool") {
            continue;
        }
        let Some(GuardrailValue::Text(call_id)) = fields.get("tool_call_id") else {
            continue;
        };
        let Some(source) = call_names.get(call_id) else {
            continue;
        };
        if lookup.output_policy(source) != POLICY_UNTRUSTED {
            continue;
        }
        if !sources.contains(source) {
            sources.push(source.clone());
        }
    }
    sources
}

fn messages_from(request_data: &BTreeMap<String, GuardrailValue>) -> &[GuardrailValue] {
    if let Some(GuardrailValue::Array(messages)) = request_data.get("messages") {
        return messages;
    }
    &[]
}

#[must_use]
pub(super) fn trust_chain_failure(
    trusted: &[String],
    request_data: &BTreeMap<String, GuardrailValue>,
    lookup: &dyn ToolPolicyLookup,
) -> Option<GuardrailFailure> {
    if trusted.is_empty() {
        return None;
    }
    let sources = untrusted_sources(messages_from(request_data), lookup);
    if sources.is_empty() {
        return None;
    }
    tracing::warn!(
        "ToolPolicyGuardrail: trust chain violation, {} require trusted input but conversation has untrusted output from {}",
        quoted_names(trusted),
        quoted_names(&sources)
    );
    let message = format!(
        "{} requires trusted input but conversation contains untrusted output from {}.",
        trusted.join(", "),
        sources.join(", ")
    );
    Some(GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail: detail(&[
            ("error", GuardrailValue::Text(ERROR_TEXT.to_owned())),
            ("blocked_tools", names_value(trusted)),
            ("untrusted_sources", names_value(&sources)),
            ("message", GuardrailValue::Text(message)),
        ]),
    })
}
