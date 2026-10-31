use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
};
use crate::grayswan::wire::{
    GraySwanResponse, GraySwanRuleNumber, GraySwanViolatedRule, GraySwanViolatedRuleFields,
};

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
    let extra = BTreeMap::from([("content".to_owned(), GuardrailExtraValue::Text(content))]);
    GuardrailMessage {
        role: role.to_owned(),
        extra,
    }
}
fn text_message_with_tool_calls(
    content: String,
    tool_calls: &[GuardrailExtraValue],
) -> GuardrailMessage {
    let mut message = tool_calls_message(tool_calls);
    message
        .extra
        .insert("content".to_owned(), GuardrailExtraValue::Text(content));
    message
}
fn tool_calls_message(tool_calls: &[GuardrailExtraValue]) -> GuardrailMessage {
    let extra = BTreeMap::from([(
        "tool_calls".to_owned(),
        GuardrailExtraValue::Array(tool_calls.to_vec()),
    )]);
    GuardrailMessage {
        role: "assistant".to_owned(),
        extra,
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
fn describe_violated_rule(rule: &GraySwanViolatedRule) -> String {
    match rule {
        GraySwanViolatedRule::Structured(fields) => describe_structured_rule(fields),
        GraySwanViolatedRule::Plain(value) => describe_extra_value(value),
    }
}
fn describe_structured_rule(fields: &GraySwanViolatedRuleFields) -> String {
    let Some(name) = fields.name.as_deref().filter(|name| !name.is_empty()) else {
        return describe_extra_value(&GuardrailExtraValue::Object(fields.to_fields()));
    };
    let prefixed_name = fields
        .rule
        .as_ref()
        .filter(|number| number.is_truthy())
        .map_or_else(
            || name.to_owned(),
            |number| format!("#{} {name}", rule_number_text(number)),
        );
    fields
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
        .map_or_else(
            || prefixed_name.clone(),
            |description| format!("{prefixed_name}: {description}"),
        )
}
fn rule_number_text(number: &GraySwanRuleNumber) -> String {
    match number {
        GraySwanRuleNumber::Integer(value) => value.to_string(),
        GraySwanRuleNumber::Text(value) => value.clone(),
    }
}
fn describe_extra_value(value: &GuardrailExtraValue) -> String {
    match value {
        GuardrailExtraValue::Text(text) => text.clone(),
        GuardrailExtraValue::Boolean(true) => "True".to_owned(),
        GuardrailExtraValue::Boolean(false) => "False".to_owned(),
        GuardrailExtraValue::SignedInteger(number) => number.to_string(),
        GuardrailExtraValue::UnsignedInteger(number) => number.to_string(),
        GuardrailExtraValue::Decimal(number) => number.to_string(),
        GuardrailExtraValue::Null => "None".to_owned(),
        GuardrailExtraValue::Array(items) => {
            let rendered = items
                .iter()
                .map(describe_extra_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        GuardrailExtraValue::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|(key, field_value)| format!("{key}: {}", describe_extra_value(field_value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{rendered}}}")
        }
    }
}
