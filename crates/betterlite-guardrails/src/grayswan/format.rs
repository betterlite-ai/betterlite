use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
};
use crate::grayswan::wire::GraySwanResponse;

pub(super) fn build_messages(
    texts: &[String],
    tool_calls: &[GuardrailExtraValue],
    input_type: GuardrailInputType,
    context: Vec<GuardrailMessage>,
) -> Vec<GuardrailMessage> {
    let role = if input_type == GuardrailInputType::Response {
        "assistant"
    } else {
        "user"
    };
    let mut messages = context;
    match texts {
        [single] if !tool_calls.is_empty() => {
            messages.push(text_message_with_tool_calls(single.clone(), tool_calls));
        }
        body => {
            messages.extend(body.iter().map(|text| text_message(role, text.clone())));
            if !tool_calls.is_empty() {
                messages.push(tool_calls_message(tool_calls));
            }
        }
    }
    messages
}

fn text_message(role: &str, content: String) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([("content".to_owned(), GuardrailExtraValue::Text(content))]),
    }
}

fn text_message_with_tool_calls(
    content: String,
    tool_calls: &[GuardrailExtraValue],
) -> GuardrailMessage {
    GuardrailMessage {
        role: "assistant".to_owned(),
        extra: BTreeMap::from([
            ("content".to_owned(), GuardrailExtraValue::Text(content)),
            (
                "tool_calls".to_owned(),
                GuardrailExtraValue::Array(tool_calls.to_vec()),
            ),
        ]),
    }
}

fn tool_calls_message(tool_calls: &[GuardrailExtraValue]) -> GuardrailMessage {
    GuardrailMessage {
        role: "assistant".to_owned(),
        extra: BTreeMap::from([(
            "tool_calls".to_owned(),
            GuardrailExtraValue::Array(tool_calls.to_vec()),
        )]),
    }
}

pub(super) fn format_violation_message(response: &GraySwanResponse, is_output: bool) -> String {
    let location = if is_output {
        "model response"
    } else {
        "request"
    };
    let mut message = format!(
        "Gray Swan Cygnal flagged the {location} with a violation score of {:.2}.",
        response.violation
    );
    if !response.violated_rule_descriptions.is_empty() {
        let rules = response
            .violated_rule_descriptions
            .iter()
            .map(describe_violated_rule)
            .collect::<Vec<_>>()
            .join(", ");
        if write!(message, " It violated rule(s): {rules}.").is_err() {
            return message;
        }
    }
    if response.mutation == Some(true) {
        message.push_str(" A mutation disguising harmful intent was detected.");
    }
    if response.ipi == Some(true) {
        message.push_str(" An indirect prompt injection was detected.");
    }
    message
}

fn describe_violated_rule(rule: &GuardrailExtraValue) -> String {
    if let GuardrailExtraValue::Object(fields) = rule {
        match (
            field_text(fields, "name"),
            field_text(fields, "description"),
        ) {
            (Some(matched_name), Some(matched_description)) => {
                return format!("{matched_name}: {matched_description}");
            }
            (Some(matched_name), None) => return matched_name,
            _ => {}
        }
    }
    if let GuardrailExtraValue::Text(text) = rule {
        return text.clone();
    }
    serde_json::to_string(rule).unwrap_or_default()
}

fn field_text(fields: &BTreeMap<String, GuardrailExtraValue>, name: &str) -> Option<String> {
    match fields.get(name)? {
        GuardrailExtraValue::Text(text) => Some(text.clone()),
        GuardrailExtraValue::Null
        | GuardrailExtraValue::Boolean(_)
        | GuardrailExtraValue::SignedInteger(_)
        | GuardrailExtraValue::UnsignedInteger(_)
        | GuardrailExtraValue::Decimal(_)
        | GuardrailExtraValue::Array(_)
        | GuardrailExtraValue::Object(_) => None,
    }
}
